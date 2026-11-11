/*
 * studio_fx.h: the studio's effects (BRYX addition; see studio.h).
 *
 *   StReverb   stereo 8-line feedback delay network (the four spaces)
 *   StDelay    stereo / ping-pong delay with filtered feedback
 *   StBiquad   RBJ-cookbook biquads (low/high-pass, peak, shelves)
 *   StComp     feed-forward compressor, stereo-linked, keyable
 *   StLimiter  lookahead peak limiter (the master's last stage)
 *
 * Conventions (studio.h): parameters in Hz, seconds and dB so behaviour
 * does not depend on the sample rate; *_init may allocate, *_process
 * never does; same input, same output bytes.
 *
 * Effects that make a NEW signal (reverb, delay) ADD it into the output
 * buffers they are given, like the voices. Effects that change a signal
 * (biquad, compressor, limiter) work in place.
 */
#ifndef BRYX_STUDIO_FX_H
#define BRYX_STUDIO_FX_H

#include "studio.h"

/* ------------------------------------------------------------------ */
/* Biquad (Robert Bristow-Johnson, "Cookbook formulae for audio EQ     */
/* biquad filter coefficients"). Stereo state, transposed direct form  */
/* II in double precision (low corner frequencies at 48 kHz are        */
/* ill-conditioned in float).                                          */

typedef struct {
    double b0, b1, b2, a1, a2; /* normalised: a0 = 1 */
    double z1[2], z2[2];       /* per-channel state */
} StBiquad;

/* The design functions set the coefficients and leave the state alone,
 * so a filter can be retuned between blocks without a click. A new
 * filter needs st_biquad_reset (or a zeroed struct) once. Frequencies
 * are clamped into (1 Hz, 0.49 * rate). */
void st_biquad_lowpass(StBiquad *b, float rate, float hz, float q);
void st_biquad_highpass(StBiquad *b, float rate, float hz, float q);
void st_biquad_peak(StBiquad *b, float rate, float hz, float q, float gain_db);
/* Shelves with RBJ slope S = 1; `hz` is the midpoint, where the gain is
 * gain_db / 2. */
void st_biquad_lowshelf(StBiquad *b, float rate, float hz, float gain_db);
void st_biquad_highshelf(StBiquad *b, float rate, float hz, float gain_db);
/* A pass-through (b0 = 1): what a disabled band is set to. */
void st_biquad_identity(StBiquad *b);
void st_biquad_reset(StBiquad *b);
/* In place; r may be NULL for a mono signal (channel 0's state). */
void st_biquad_process(StBiquad *b, float *l, float *r, int n);

/* ------------------------------------------------------------------ */
/* Reverb: an 8-line FDN (see studio_fx.c for the design and sources). */

#define ST_FDN_LINES 8
#define ST_ER_TAPS 6
#define ST_FDN_LFOS 4

typedef struct {
    float rate;
    StReverbParams p;
    /* input: high-pass, then a stereo pre-delay line the early
     * reflections tap */
    StBiquad low_cut;
    float *pre_l, *pre_r;
    int pre_cap, pre_w, pre_delay;
    int er_tap_l[ST_ER_TAPS], er_tap_r[ST_ER_TAPS]; /* frames after pre_delay */
    float er_gain[ST_ER_TAPS];
    /* the network */
    float *line[ST_FDN_LINES];
    int line_cap[ST_FDN_LINES], line_len[ST_FDN_LINES], line_w[ST_FDN_LINES];
    float fb_gain[ST_FDN_LINES];  /* g_i: decay per pass at low frequency */
    float damp_a[ST_FDN_LINES];   /* one-pole low-pass pole per line */
    float damp_b[ST_FDN_LINES];   /* g_i * (1 - a_i) */
    float damp_z[ST_FDN_LINES];   /* its state */
    /* slow sine LFOs on the four longest lines (rotating phasors) */
    double lfo_c[ST_FDN_LFOS], lfo_s[ST_FDN_LFOS];
    double lfo_rc[ST_FDN_LFOS], lfo_rs[ST_FDN_LFOS];
    float mod_samples;            /* peak excursion, frames */
    float out_gain;               /* late-tail normalisation */
    float width;
    float *mem;                   /* one allocation behind every buffer */
} StReverb;

/* 0 ok, -1 on bad arguments or allocation failure. */
int st_reverb_init(StReverb *r, float rate, const StReverbParams *p);
void st_reverb_free(StReverb *r);
/* ADDS the wet signal into out_l/out_r; in and out must not alias. */
void st_reverb_process(StReverb *r, const float *in_l, const float *in_r,
                       float *out_l, float *out_r, int n);
/* st_reverb_preset is declared in studio.h. */

/* ------------------------------------------------------------------ */
/* Delay: stereo or ping-pong, filtered feedback, fractional times.    */

typedef struct {
    float rate;
    StDelayParams p;
    float *buf_l, *buf_r;
    int cap, w;
    double cur_l, cur_r;          /* delay in frames, gliding toward... */
    double tgt_l, tgt_r;          /* ...these (st_delay_set_params) */
    double glide;                 /* one-pole coefficient for the glide */
    StBiquad low_cut, high_cut;   /* in the feedback loop */
    float *mem;
} StDelay;

/* max_seconds bounds every later time (allocated here). 0 ok, -1 error. */
int st_delay_init(StDelay *d, float rate, const StDelayParams *p, float max_seconds);
void st_delay_free(StDelay *d);
/* New times / feedback / filters (a tempo ramp). Times glide to the new
 * values over ~20 ms, so changing them per block does not click. */
void st_delay_set_params(StDelay *d, const StDelayParams *p);
/* ADDS the repeats (no dry signal) into out_l/out_r; in and out must not
 * alias. The first repeat is at time_l (and time_r) at unity gain; each
 * later one is `feedback` times the previous, through the loop filters. */
void st_delay_process(StDelay *d, const float *in_l, const float *in_r,
                      float *out_l, float *out_r, int n);

/* ------------------------------------------------------------------ */
/* Compressor: feed-forward, log-domain smooth decoupled peak detector */
/* (Giannoulis, Massberg & Reiss 2012), stereo-linked.                 */

typedef struct {
    float rate;
    StCompParams p;
    double att_coef, rel_coef;
    double hold_db;               /* peak-held gain reduction, dB */
    double gr_db;                 /* smoothed gain reduction, dB >= 0 */
} StComp;

void st_comp_init(StComp *c, float rate, const StCompParams *p);
/* In place. key_l/key_r: an external sidechain, or NULL to key on l/r
 * themselves (key_r NULL with key_l set keys on key_l alone). The level
 * is the sample peak max(|key_l|, |key_r|), so a steady sine at -10 dBFS
 * (peak) reads -10 dB. */
void st_comp_process(StComp *c, float *l, float *r, const float *key_l,
                     const float *key_r, int n);
/* Current gain reduction in dB, as a positive number (makeup excluded). */
float st_comp_gain_reduction_db(const StComp *c);

/* ------------------------------------------------------------------ */
/* Lookahead limiter.                                                  */

typedef struct {
    float rate;
    float ceiling;                /* linear */
    int look;                     /* lookahead = latency, frames */
    double rel_coef;
    /* audio delay line (look frames, per channel) */
    float *dl_l, *dl_r;
    int dl_pos;
    /* sliding minimum of the required gain over look + 1 frames: a
     * monotonic deque in a ring of look + 1 slots */
    long long *dq_idx;
    float *dq_val;
    int dq_head, dq_count;
    long long t;                  /* frames processed */
    /* release-smoothed gain, then a box average over look + 1 frames */
    double env;
    double *box;
    double box_sum;
    int box_pos;
    long long clamps;             /* last-resort clamps (should stay 0) */
    void *mem;
} StLimiter;

/* ceiling_db (e.g. -1), lookahead and release in seconds. 0 ok, -1 error. */
int st_limiter_init(StLimiter *lm, float rate, float ceiling_db, float lookahead,
                    float release);
void st_limiter_free(StLimiter *lm);
/* In place; the output is the input delayed by st_limiter_latency frames
 * with its sample peaks held at or below the ceiling. */
void st_limiter_process(StLimiter *lm, float *l, float *r, int n);
int st_limiter_latency(const StLimiter *lm);

#endif
