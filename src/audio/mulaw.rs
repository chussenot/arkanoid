// G.711 mu-law: how BRYX stores its rendered cues, one byte a sample.
//
// The engine renders at 16-bit precision. Linear 8-bit storage would keep
// only its top byte, so a quiet passage (a break, a fading tail) would
// sound at a few bits. Mu-law spends the same byte logarithmically: about
// 13 bits of range, with a noise floor that follows the signal down, for
// the same 22 MB of embedded audio.
//
// build.rs `include!`s this file to encode; src/audio/cues.rs decodes. One
// file, so the two can never disagree. (Plain comments, not `//!`: an
// included file cannot carry inner doc comments.)

const MULAW_BIAS: i32 = 0x84;
const MULAW_CLIP: i32 = 32_635;

/// One 16-bit sample as mu-law. Silence is 0xFF.
#[allow(dead_code)] // only build.rs encodes
pub fn mulaw_encode(sample: i16) -> u8 {
    let mut s = i32::from(sample);
    let sign = if s < 0 {
        s = -s;
        0x80
    } else {
        0
    };
    s = s.min(MULAW_CLIP) + MULAW_BIAS;
    let mut exponent = 7;
    let mut mask = 0x4000;
    while exponent > 0 && s & mask == 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = (s >> (exponent + 3)) & 0x0F;
    !((sign | (exponent << 4) | mantissa) as u8)
}

/// One mu-law byte as its 16-bit sample.
pub const fn mulaw_decode(byte: u8) -> i16 {
    let u = !byte;
    let exponent = (u >> 4) & 0x07;
    let mantissa = (u & 0x0F) as i32;
    let magnitude = (((mantissa << 3) + MULAW_BIAS) << exponent) - MULAW_BIAS;
    if u & 0x80 != 0 {
        -magnitude as i16
    } else {
        magnitude as i16
    }
}

/// Every mu-law byte as a sample in `-1.0..1.0`: what the mixer reads.
#[allow(dead_code)] // build.rs includes this file too, and never decodes
pub static MULAW_TO_F32: [f32; 256] = {
    let mut table = [0.0; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = mulaw_decode(i as u8) as f32 / 32_768.0;
        i += 1;
    }
    table
};
