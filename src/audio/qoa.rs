//! A streaming decoder for QOA, "the Quite OK Audio Format"
//! (<https://qoaformat.org>), the format BRYX's music is stored in
//! (`docs/design/orchestral-score.md`, 4.4).
//!
//! `build.rs` renders the score to 48 kHz stereo and encodes it with the
//! reference encoder (`audio/vendor/qoa.h`, MIT); this decodes it inside
//! the audio callback. It is written for that callback: no allocation, no
//! locks, and a fixed amount of work per frame. It decodes one slice (20
//! samples per channel) at a time, so the cost never spikes, and reads the
//! bytes straight from the binary.
//!
//! A port of `qoa_decode_frame`, checked bit for bit against the
//! reference decoder by the tests below.

/// Samples per channel in one slice.
const SLICE_LEN: usize = 20;
/// Channels BRYX stores (and the most this decoder handles).
pub const MAX_CHANNELS: usize = 2;

/// `dequant_tab[scalefactor][quantized]`, from the reference.
const DEQUANT: [[i32; 8]; 16] = [
    [1, -1, 3, -3, 5, -5, 7, -7],
    [5, -5, 18, -18, 32, -32, 49, -49],
    [16, -16, 53, -53, 95, -95, 147, -147],
    [34, -34, 113, -113, 203, -203, 315, -315],
    [63, -63, 210, -210, 378, -378, 588, -588],
    [104, -104, 345, -345, 621, -621, 966, -966],
    [158, -158, 528, -528, 950, -950, 1477, -1477],
    [228, -228, 760, -760, 1368, -1368, 2128, -2128],
    [316, -316, 1053, -1053, 1895, -1895, 2947, -2947],
    [422, -422, 1405, -1405, 2529, -2529, 3934, -3934],
    [548, -548, 1828, -1828, 3290, -3290, 5117, -5117],
    [696, -696, 2320, -2320, 4176, -4176, 6496, -6496],
    [868, -868, 2893, -2893, 5207, -5207, 8099, -8099],
    [1064, -1064, 3548, -3548, 6386, -6386, 9933, -9933],
    [1286, -1286, 4288, -4288, 7718, -7718, 12005, -12005],
    [1536, -1536, 5120, -5120, 9216, -9216, 14336, -14336],
];

/// What is wrong with a QOA stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoaError {
    Truncated,
    NotQoa,
    /// Streaming QOA (a file header with zero samples) is not used here.
    Unsized,
    /// More channels than [`MAX_CHANNELS`], or a frame disagreeing with
    /// the first one.
    Unsupported,
}

#[derive(Debug, Clone, Copy, Default)]
struct Lms {
    history: [i32; 4],
    weights: [i32; 4],
}

impl Lms {
    fn predict(&self) -> i32 {
        let p: i32 = (0..4).map(|i| self.weights[i] * self.history[i]).sum();
        p >> 13
    }

    fn update(&mut self, sample: i32, residual: i32) {
        let delta = residual >> 4;
        for i in 0..4 {
            self.weights[i] += if self.history[i] < 0 { -delta } else { delta };
        }
        self.history.copy_within(1..4, 0);
        self.history[3] = sample;
    }
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, QoaError> {
    let b = bytes.get(at..at + 8).ok_or(QoaError::Truncated)?;
    Ok(u64::from_be_bytes(b.try_into().unwrap()))
}

/// A QOA stream being decoded, frame by frame, one slice at a time.
#[derive(Debug, Clone)]
pub struct QoaStream<'a> {
    bytes: &'a [u8],
    channels: usize,
    rate: u32,
    samples: u32,
    /// Byte offset of the next slice (or frame header).
    cursor: usize,
    /// Samples per channel left in the current frame; 0 means a frame
    /// header is next.
    frame_left: usize,
    lms: [Lms; MAX_CHANNELS],
    slice: [[i16; SLICE_LEN]; MAX_CHANNELS],
    slice_len: usize,
    slice_pos: usize,
    /// Samples per channel handed out since the start.
    position: u32,
}

impl<'a> QoaStream<'a> {
    /// Opens a QOA file held in memory. Reads the file and first frame
    /// headers; decodes nothing yet.
    pub fn new(bytes: &'a [u8]) -> Result<Self, QoaError> {
        let header = read_u64(bytes, 0)?;
        if (header >> 32) as u32 != 0x716f_6166 {
            return Err(QoaError::NotQoa);
        }
        let samples = (header & 0xffff_ffff) as u32;
        if samples == 0 {
            return Err(QoaError::Unsized);
        }
        let first = read_u64(bytes, 8)?;
        let channels = ((first >> 56) & 0xff) as usize;
        let rate = ((first >> 32) & 0xff_ffff) as u32;
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(QoaError::Unsupported);
        }
        Ok(Self {
            bytes,
            channels,
            rate,
            samples,
            cursor: 8,
            frame_left: 0,
            lms: [Lms::default(); MAX_CHANNELS],
            slice: [[0; SLICE_LEN]; MAX_CHANNELS],
            slice_len: 0,
            slice_pos: 0,
            position: 0,
        })
    }

    #[cfg(test)]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Samples per channel handed out so far (tests).
    #[cfg(test)]
    pub fn position(&self) -> u32 {
        self.position
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Samples per channel in the whole stream (never 0: `new` refuses an
    /// unsized stream).
    #[cfg(test)]
    pub fn len(&self) -> u32 {
        self.samples
    }

    /// Back to the first sample (a music loop's wrap).
    pub fn rewind(&mut self) {
        self.cursor = 8;
        self.frame_left = 0;
        self.slice_len = 0;
        self.slice_pos = 0;
        self.position = 0;
    }

    /// The next sample of every channel, as 16-bit values (a mono stream
    /// repeats its channel), or `None` at the end of the stream.
    pub fn next_frame(&mut self) -> Option<[i16; 2]> {
        if self.position >= self.samples {
            return None;
        }
        if self.slice_pos == self.slice_len && self.decode_slice().is_err() {
            return None;
        }
        let i = self.slice_pos;
        self.slice_pos += 1;
        self.position += 1;
        let l = self.slice[0][i];
        let r = if self.channels > 1 {
            self.slice[1][i]
        } else {
            l
        };
        Some([l, r])
    }

    fn read_frame_header(&mut self) -> Result<(), QoaError> {
        let header = read_u64(self.bytes, self.cursor)?;
        let channels = ((header >> 56) & 0xff) as usize;
        let rate = ((header >> 32) & 0xff_ffff) as u32;
        let frame_samples = ((header >> 16) & 0xffff) as usize;
        if channels != self.channels || rate != self.rate || frame_samples == 0 {
            return Err(QoaError::Unsupported);
        }
        self.cursor += 8;
        for c in 0..self.channels {
            let mut history = read_u64(self.bytes, self.cursor)?;
            let mut weights = read_u64(self.bytes, self.cursor + 8)?;
            self.cursor += 16;
            for i in 0..4 {
                self.lms[c].history[i] = i32::from((history >> 48) as i16);
                self.lms[c].weights[i] = i32::from((weights >> 48) as i16);
                history <<= 16;
                weights <<= 16;
            }
        }
        self.frame_left = frame_samples;
        Ok(())
    }

    fn decode_slice(&mut self) -> Result<(), QoaError> {
        if self.frame_left == 0 {
            self.read_frame_header()?;
        }
        let len = self.frame_left.min(SLICE_LEN);
        for c in 0..self.channels {
            let mut slice = read_u64(self.bytes, self.cursor)?;
            self.cursor += 8;
            let scalefactor = ((slice >> 60) & 0xf) as usize;
            slice <<= 4;
            let lms = &mut self.lms[c];
            for s in self.slice[c].iter_mut().take(len) {
                let predicted = lms.predict();
                let quantized = ((slice >> 61) & 0x7) as usize;
                let dequantized = DEQUANT[scalefactor][quantized];
                let reconstructed = (predicted + dequantized).clamp(-32768, 32767);
                *s = reconstructed as i16;
                slice <<= 3;
                lms.update(reconstructed, dequantized);
            }
        }
        self.frame_left -= len;
        self.slice_len = len;
        self.slice_pos = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12,000 stereo samples (two full 5,120-sample frames and a partial
    /// one), encoded by the reference encoder, and the reference decoder's
    /// output, both made by `audio/vendor/qoa.h`.
    const REF_QOA: &[u8] = include_bytes!("testdata/ref.qoa");
    const REF_PCM: &[u8] = include_bytes!("testdata/ref.s16");

    fn reference() -> Vec<i16> {
        REF_PCM
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b))
            .collect()
    }

    #[test]
    fn decodes_bit_for_bit_like_the_reference() {
        let mut s = QoaStream::new(REF_QOA).unwrap();
        assert_eq!(
            (s.channels(), s.sample_rate(), s.len()),
            (2, 48_000, 12_000)
        );
        let mut out = Vec::new();
        while let Some([l, r]) = s.next_frame() {
            out.push(l);
            out.push(r);
        }
        assert_eq!(out, reference());
    }

    #[test]
    fn rewinding_replays_the_same_samples() {
        let mut s = QoaStream::new(REF_QOA).unwrap();
        let first: Vec<_> = (0..7_000).map(|_| s.next_frame().unwrap()).collect();
        while s.next_frame().is_some() {}
        assert_eq!(s.next_frame(), None, "ends after its samples");
        s.rewind();
        let again: Vec<_> = (0..7_000).map(|_| s.next_frame().unwrap()).collect();
        assert_eq!(first, again);
    }

    #[test]
    fn rejects_what_is_not_a_whole_qoa_file() {
        assert_eq!(QoaStream::new(b"RIFF....").unwrap_err(), QoaError::NotQoa);
        assert_eq!(
            QoaStream::new(&REF_QOA[..4]).unwrap_err(),
            QoaError::Truncated
        );
        let mut s = QoaStream::new(&REF_QOA[..200]).unwrap();
        let decoded = std::iter::from_fn(|| s.next_frame()).count();
        assert!(
            decoded < 12_000,
            "a truncated file stops early, never panics"
        );
    }
}
