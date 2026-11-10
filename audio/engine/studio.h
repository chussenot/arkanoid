/*
 * studio.h: BRYX's float stereo renderer (BRYX addition to the Electric
 * Pulse fork; see PROVENANCE.md and docs/design/orchestral-score.md).
 *
 * The ABC path (abc.c, audio_mix.c) renders the existing cues and stays
 * as it is. The studio renders `.score` files: timed notes in beats on
 * named instruments (sampled orchestra or analog-style synth), group
 * buses, aux returns (reverb, delay) and a master with a limiter, in
 * 32-bit float stereo at 48 kHz.
 *
 * Modules, one owner each:
 *   studio_synth.{h,c}   StSynthVoice: the analog-style voice
 *   studio_sampler.{h,c} StSampleSet, StSamplerVoice: SFZ-subset sampler
 *   studio_fx.{h,c}      StReverb, StDelay, StBiquad, StComp, StLimiter
 *   studio.c             the .score parser, scheduler, mixer and render
 *
 * Rules for every module:
 *   - Deterministic: the same input renders the same bytes. Randomness
 *     only from st_rand (a seeded xorshift), never rand() or time.
 *   - Build-time only: allocation is fine in *_init / *_load, never per
 *     sample.
 *   - Frequencies and times in Hz and seconds, gains linear unless a name
 *     says _db, so nothing depends on the sample rate implicitly.
 *   - Voices and effects ADD into the buffers they are given; the caller
 *     zeroes them.
 */
#ifndef BRYX_STUDIO_H
#define BRYX_STUDIO_H

#include <stddef.h>
#include <stdint.h>

#define ST_DEFAULT_RATE 48000
/* Frames per processing block: modulation (automation, swells) is given
 * per block and interpolated linearly across it. */
#define ST_BLOCK 32

/* ------------------------------------------------------------------ */
/* Shared helpers (studio.c defines them).                             */

/* xorshift32; *state must start non-zero. Returns 0..2^32-1. */
uint32_t st_rand(uint32_t *state);
/* Uniform in [-1, 1). */
float st_randf(uint32_t *state);
float st_db_to_gain(float db);
float st_midi_to_hz(float note);

/* An ADSR in seconds; sustain is a level 0..1. Exponential segments:
 * attack rises toward 1.0 with an RC curve aimed slightly above it,
 * decay and release fall exponentially toward their target (time = the
 * time to cover ~99% of the way). */
typedef struct {
    float attack, decay, sustain, release;
} StAdsr;

typedef struct {
    int stage;     /* 0 idle, 1 attack, 2 decay, 3 sustain, 4 release */
    float level;
} StEnv;

void st_env_on(StEnv *e);
void st_env_off(StEnv *e);
/* One sample; returns the level. Reaching ~0 in release ends it (stage 0). */
float st_env_next(StEnv *e, const StAdsr *a, float rate);

/* Per-block modulation from the mixer (automation lanes, expression).
 * gain0 -> gain1 ramps linearly over the block; the rest are constant
 * for the block. */
typedef struct {
    float gain0, gain1;    /* linear */
    float cutoff_oct;      /* added to the filter cutoff, octaves */
    float pitch_cents;     /* added to the pitch */
} StVoiceMod;

/* ------------------------------------------------------------------ */
/* Synth patch (filled by the .score parser, played by studio_synth).  */

typedef enum {
    ST_WAVE_SAW = 0, ST_WAVE_SQUARE, ST_WAVE_PULSE, ST_WAVE_TRIANGLE,
    ST_WAVE_SINE, ST_WAVE_NOISE
} StWave;

typedef enum {
    ST_FILTER_NONE = 0, ST_FILTER_LP, ST_FILTER_HP, ST_FILTER_BP,
    ST_FILTER_LADDER          /* 4-pole resonant low-pass */
} StFilterMode;

typedef enum {
    ST_LFO_SINE = 0, ST_LFO_TRIANGLE, ST_LFO_SQUARE, ST_LFO_SAMPLE_HOLD
} StLfoShape;

typedef struct {
    StLfoShape shape;
    float rate_hz;
    float delay;           /* seconds before it fades in (over the same time) */
    float to_pitch_cents;
    float to_cutoff_oct;
    float to_amp;          /* 0..1: depth of amplitude dip */
    float to_pan;          /* 0..1 */
    float to_pulse_width;  /* 0..0.45 */
} StLfo;

typedef struct {
    StWave wave;
    float pulse_width;     /* 0.05..0.95, for ST_WAVE_PULSE */
    int stack;             /* 1..7 detuned copies */
    float detune_cents;    /* spread between the outermost copies */
    float stereo_spread;   /* 0..1: copies panned across the field */
    float sub_level;       /* 0..1: a sine one octave down */
    float noise_level;     /* 0..1: white noise added */
    StAdsr amp;
    StFilterMode filter;
    float cutoff_hz;
    float resonance;       /* 0..1 (1 = self-oscillation edge) */
    float key_track;       /* 0..1: cutoff follows the note */
    float env_amount_oct;  /* filter envelope depth, octaves (may be < 0) */
    StAdsr filter_env;
    float velocity_to_cutoff_oct; /* extra cutoff at full velocity */
    float drive;           /* 0..1: soft saturation before the filter */
    float pitch_env_semis; /* a note starts this far off and sweeps home */
    float pitch_env_time;  /* seconds */
    float glide_time;      /* seconds; legato glide when > 0 */
    int mono;              /* 1: one voice per instrument, legato */
    StLfo lfo[2];
} StSynthPatch;

void st_synth_patch_default(StSynthPatch *p);

/* ------------------------------------------------------------------ */
/* Sampler patch (instrument-level settings on top of the SFZ map).    */

typedef struct {
    StAdsr amp;            /* attack/decay/sustain/release over the sample */
    float velocity_curve;  /* gain = (velocity/127)^curve; 0 -> 1.7 */
    float start_offset;    /* seconds skipped at the start (softer attacks) */
    float tune_cents;
    int stack;             /* 1..4 copies (a solo player -> a section) */
    float stack_detune_cents;
    float stack_delay;     /* seconds of spread between the copies' onsets */
    float stereo_spread;   /* 0..1 for stacked copies */
} StSamplerPatch;

void st_sampler_patch_default(StSamplerPatch *p);

/* ------------------------------------------------------------------ */
/* Effects parameter blocks (studio_fx).                               */

typedef struct {
    float rt60;            /* seconds of decay to -60 dB */
    float predelay;        /* seconds */
    float size;            /* 0..1: delay-line lengths (room scale) */
    float damping_hz;      /* high-frequency loss in the tail */
    float low_cut_hz;      /* high-pass on the input */
    float width;           /* 0..1 stereo width of the return */
    float mod_depth;       /* 0..1 gentle delay-line modulation */
} StReverbParams;

/* room, stage, hall, long (the four spaces of the design). */
int st_reverb_preset(const char *name, StReverbParams *out);

typedef struct {
    float time_l, time_r;  /* seconds (the parser converts beats) */
    float feedback;        /* 0..0.95 */
    int ping_pong;         /* 1: the repeats alternate sides */
    float low_cut_hz;      /* in the feedback path */
    float high_cut_hz;     /* in the feedback path */
} StDelayParams;

typedef struct {
    float threshold_db;
    float ratio;           /* >= 1 */
    float attack, release; /* seconds */
    float knee_db;
    float makeup_db;
} StCompParams;

/* ------------------------------------------------------------------ */
/* Rendering a .score (studio.c).                                      */

typedef struct {
    float *frames;         /* interleaved stereo, frame_count * 2 floats */
    int frame_count;
    int rate;
} StRender;

/* Render the .score at `path` at `rate` Hz (0 = the score's own `rate`,
 * else ST_DEFAULT_RATE). Returns 0 on success; on failure fills err. */
int st_render_score(const char *path, int rate, StRender *out, char *err, size_t errlen);
void st_render_free(StRender *r);

#endif
