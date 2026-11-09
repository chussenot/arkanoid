#ifndef ELECTRIC_PULSE_AUDIO_SEQ_H
#define ELECTRIC_PULSE_AUDIO_SEQ_H

#include <stdint.h>
#include "audio_dsp.h"

/* BRYX: 12 tracks and 64 patterns of 64 steps (256 bars), from 8 and 16. */
#define SEQ_MAX_STEPS        64
#define SEQ_MAX_TRACKS       12
#define SEQ_MAX_PATTERNS     64
#define SEQ_MAX_ARRANGEMENT  64
#define SEQ_MAX_INSTRUMENTS  12
#define SEQ_MAX_FX_BUSES     4
#define SEQ_MAX_TIMELINE_STEPS (SEQ_MAX_STEPS * SEQ_MAX_ARRANGEMENT)

#define SEQ_NOTE_REST (-1)

typedef struct {
    int enabled;
    int delay_steps;
    int delay_feedback;
    int delay_mix;
    int drive_amount;
    int lowpass_amount;
    int sidechain_amount;
    int sidechain_release_ms;
    int mix_percent;
    int ladder_amount;     /* wet mix 0..100; 0 disables ladder */
    int ladder_cutoff;     /* 1..100 (percent of Nyquist) */
    int ladder_resonance;  /* 0..100 */
    /* BRYX additions (zero keeps the original behavior): */
    int insert;            /* 1: the bus replaces its voices' dry signal instead of adding to it */
    int sidechain_key;     /* track index + 1 whose note-ons trigger this bus's sidechain; the
                              keyed sidechain ducks this bus only, never the master */
    int reverb_mix;        /* BRYX: reverb added to the bus, 0..100 (0 = none) */
    int reverb_size;       /* BRYX: room size 0..100 */
    int reverb_damp;       /* BRYX: high-frequency damping 0..100 */
} SeqFxBus;

typedef struct {
    int waveform;
    int amplitude;
    int pulse_width;
    DspEnvelope envelope;
    int detune_cents;
    int vibrato_depth_cents;
    int vibrato_rate;
    int pwm_depth;
    int pwm_rate;
    int glide_ms;
    int noise_mode;
    int stacked_voices;
    int fx_send;
    int accent_gain;
    int vibrato_delay_ms;  /* BRYX: vibrato depth fades in over this long after a note-on */
    int pitch_env_cents;   /* BRYX: a note starts this far off its pitch and sweeps home... */
    int pitch_env_ms;      /* BRYX: ...over this long (a chip kick, a tom, a zap) */
    int noise_clock;       /* BRYX: 0 white, 1 long LFSR, 2 metal LFSR (see DspOscillator) */
    int bandlimit;         /* BRYX: PolyBLEP square and pulse (see DspOscillator) */
} SeqInstrument;

typedef struct {
    int16_t note;
    uint8_t velocity;
    uint8_t gate;
    uint8_t accent;
    uint8_t fx_trigger;
    uint8_t tie;           /* BRYX: 1 continues the previous step's note instead of striking a new one */
    int16_t note2;         /* BRYX: a chord's second and third notes; 0 or less = none (MIDI 0 is never written) */
    int16_t note3;
} SeqStep;

typedef struct {
    int instrument;
    int8_t automation[SEQ_MAX_STEPS];
    SeqStep steps[SEQ_MAX_STEPS];
} SeqTrack;

typedef struct {
    int length;
    int track_count;
    SeqTrack tracks[SEQ_MAX_TRACKS];
} SeqPattern;

/* BRYX: a parameter moving over time. Between start_step and end_step
 * (absolute timeline steps) the value runs from `from` to `to`; after
 * end_step it stays at `to` until another ramp moves it. */
typedef enum {
    SEQ_RAMP_LADDER_CUTOFF = 0,
    SEQ_RAMP_LADDER_RESONANCE,
    SEQ_RAMP_LOWPASS,
    SEQ_RAMP_DELAY_MIX,
    SEQ_RAMP_DRIVE,
    SEQ_RAMP_BUS_MIX,
    SEQ_RAMP_DUTY,           /* a track's pulse width, taken at each note-on */
    SEQ_RAMP_REVERB_MIX,     /* BRYX phase 3 */
    SEQ_RAMP_TEMPO           /* BRYX phase 3: the song's BPM, step by step (the timeline) */
} SeqRampParam;

#define SEQ_MAX_RAMPS 32

typedef struct {
    int track;               /* SEQ_RAMP_DUTY: track index */
    int bus;                 /* the other params: FX bus index */
    SeqRampParam param;
    int from;
    int to;
    int start_step;
    int end_step;
} SeqRamp;

typedef struct {
    char title[128];
    int tempo_bpm;
    int swing_pct;
    int steps_per_beat;
    int instrument_count;
    SeqInstrument instruments[SEQ_MAX_INSTRUMENTS];
    int pattern_count;
    SeqPattern patterns[SEQ_MAX_PATTERNS];
    int arrangement_length;
    uint8_t arrangement[SEQ_MAX_ARRANGEMENT];
    int fx_bus_count;
    SeqFxBus fx_buses[SEQ_MAX_FX_BUSES];
    int master_permille;   /* BRYX: output gain in 1/1000ths; 0 means 1000 (unity) */
    int chord_voices;      /* BRYX: 1 when any track writes chords (more voices per track) */
    int loop;              /* BRYX: %%loop on: render twice, keep the second pass, so the
                              seam carries the end's tails into the start */
    int ramp_count;        /* BRYX */
    SeqRamp ramps[SEQ_MAX_RAMPS];
} SeqSong;

typedef struct {
    int pattern_index;
    int pattern_step;
    int samples;
} SeqTimelineStep;

typedef struct {
    int total_steps;
    int total_samples;
    int max_track_count;
    SeqTimelineStep steps[SEQ_MAX_TIMELINE_STEPS];
} SeqTimeline;

typedef struct {
    int active;
    int track_index;
    int instrument_index;
    int note;
    int velocity;
    int accent;
    int fx_trigger;
    int gate_percent;
    int duration_samples;
    int step_index;
    int note2;             /* BRYX: chord notes; 0 or less = none */
    int note3;
} SeqNoteEvent;

#define SEQ_MAX_STEP_EVENTS SEQ_MAX_TRACKS

int seq_song_total_steps(const SeqSong *song);
int seq_compile_timeline(const SeqSong *song, int sample_rate, SeqTimeline *timeline);
/* BRYX: the samples a note struck at absolute_step on track_index lasts,
 * through every tied step after it. */
int seq_held_samples(const SeqSong *song, const SeqTimeline *timeline,
                     int absolute_step, int track_index);
int seq_collect_step_events(const SeqSong *song, const SeqTimeline *timeline,
                            int absolute_step, SeqNoteEvent events[SEQ_MAX_STEP_EVENTS]);

#endif
