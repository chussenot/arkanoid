/*
 * studio_synth.h: the studio's analog-style synth voice. BRYX addition
 * (see studio.h).
 *
 * One StSynthVoice plays one note of an StSynthPatch: up to seven detuned
 * band-limited oscillators plus a sub oscillator and noise, a drive stage,
 * a per-voice filter (zero-delay-feedback SVF or a 4-pole ladder), amp,
 * filter and pitch envelopes, legato glide and two free-running LFOs.
 *
 * The voice is a plain struct (no heap); st_synth_start fills it. It keeps
 * a pointer to the patch, which must outlive it; the patch is read at
 * control rate, so the voice follows edits to it.
 *
 * Signal flow, per sample:
 *
 *   stack of oscillators (panned across stereo_spread) + sub + noise
 *     -> drive (soft saturation) -> filter -> amp envelope x velocity
 *     x LFO amp dip x StVoiceMod gain -> LFO pan -> ADD into l/r
 *
 * Modulation (pitch, cutoff, LFOs, filter and pitch envelopes, glide) is
 * computed every ST_SYNTH_CTRL samples and ramped linearly in between, so
 * nothing zippers and the cost stays low. The control clock runs across
 * st_synth_render calls, so how a render is split into calls does not
 * change the output (only the StVoiceMod values a call carries do).
 *
 * Level: a single centred oscillator at full velocity comes out at peak
 * 1.0 on each channel (the equal-power pan law is normalised so centre is
 * unity); a stack of N copies is scaled by 1/sqrt(N).
 */
#ifndef BRYX_STUDIO_SYNTH_H
#define BRYX_STUDIO_SYNTH_H

#include <stdint.h>

#include "studio.h"

#define ST_SYNTH_MAX_STACK 7
/* Samples per control-rate step (modulation is ramped across it). */
#define ST_SYNTH_CTRL 8

typedef struct {
    const StSynthPatch *patch;
    float rate;
    int active;             /* 1 from start until the amp envelope ends */
    int primed;             /* 0: the next control step jumps, no ramp */
    int ctrl_left;          /* samples left in the current control step */
    int stack;              /* copies in use, 1..ST_SYNTH_MAX_STACK */
    int stereo;             /* 1: copies panned apart, filter runs twice */
    uint32_t rng;           /* noise and sample & hold */

    float velocity;         /* 0..1 */
    float note_target;      /* semitones (MIDI note, fractional) */
    float note_cur;         /* gliding toward note_target */
    float pitch_env;        /* semitones of pitch-envelope offset left */
    StEnv amp_env, filt_env;

    /* Oscillators: phases in cycles [0, 1), double so minutes of
     * accumulation do not drift; ratio = the copy's detune. */
    double phase[ST_SYNTH_MAX_STACK];
    double ratio[ST_SYNTH_MAX_STACK];
    float gain_l[ST_SYNTH_MAX_STACK], gain_r[ST_SYNTH_MAX_STACK];
    double sub_phase;

    /* LFOs. */
    float lfo_phase[2];
    float lfo_hold[2];      /* the current sample & hold value */
    float lfo_time;         /* seconds since the note started */

    /* Control-rate values and their per-sample ramps. */
    double inc, inc_step;   /* base phase increment, cycles per sample */
    float g, g_step;        /* filter prewarped gain tan(pi fc / fs) */
    float amp, amp_step;    /* velocity x LFO amp dip */
    float pan_l, pan_l_step, pan_r, pan_r_step;
    float pw, pw_step;      /* pulse width for square / pulse */

    /* Filter state: [channel][ic1eq, ic2eq] for the SVF, [channel][s1..s4]
     * for the ladder. */
    float flt[2][4];
} StSynthVoice;

/* Start a note. velocity 0..1; seed drives the copies' start phases, the
 * LFO phases and the noise, so the same seed renders the same bytes. */
void st_synth_start(StSynthVoice *v, const StSynthPatch *p, float rate,
                    float note, float velocity, uint32_t seed);
/* Mono/legato: move to a new note without retriggering envelopes, gliding
 * over p->glide_time (0 = jump). A voice in its release re-enters attack
 * from its current level; an inactive voice is left alone. */
void st_synth_legato(StSynthVoice *v, float note, float velocity);
/* Note off: amp and filter envelopes enter their release. */
void st_synth_release(StSynthVoice *v);
/* ADD n stereo frames into l/r. mod may be NULL (unity gain, no offsets).
 * Returns 1 while sounding, 0 once the amp envelope has finished (the
 * frames after that point are left untouched). */
int st_synth_render(StSynthVoice *v, const StVoiceMod *mod, float *l,
                    float *r, int n);
int st_synth_active(const StSynthVoice *v);

#endif
