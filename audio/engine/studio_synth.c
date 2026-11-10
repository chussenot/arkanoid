/*
 * studio_synth.c: the studio's analog-style synth voice. BRYX addition
 * (see studio.h and studio_synth.h).
 *
 * DSP choices, and where the forms come from:
 *
 *  - Saw and square/pulse: naive waveforms corrected at each step with a
 *    four-sample polynomial BLEP residual, the integrated cubic B-spline
 *    (Valimaki, Pekonen and Nam, JASA 2012). The two-sample residual
 *    audio_dsp.c uses (Valimaki and Huovilainen, IEEE SPM 2007) left a
 *    3520 Hz saw's aliasing only 24.5 dB under its harmonics; this one
 *    gets 33.5 dB, for a slightly softer top octave.
 *  - Triangle: the naive triangle corrected at its two corners with the
 *    matching polynomial BLAMP, the integral of that BLEP residual
 *    (Esqueda, Valimaki and Bilbao, "Rounding corners with BLAMP", DAFx
 *    2016). Chosen over a leaky integrator of a BLEP square because it is
 *    stateless: no DC offset or amplitude that depends on the pitch, and
 *    no settling when the pitch glides.
 *  - LP/HP/BP: the trapezoidal-integrated state-variable filter from
 *    Andrew Simper (Cytomic), "Linear trapezoidal integrated SVF" (2013):
 *    zero-delay feedback, tuned by g = tan(pi fc / fs), stable under fast
 *    modulation. For LP/HP the band integrator's state saturates softly
 *    above a knee (exactly linear below it) so high resonance stays
 *    bounded, as an analog SVF's integrators would.
 *  - Ladder: Zavalishin's TPT 4-pole ladder ("The Art of VA Filter
 *    Design", ch. 5): four trapezoidal one-poles in series with the global
 *    feedback solved in closed form, then a tanh saturator at the summing
 *    junction (the "solve linear, then saturate" approach of the same
 *    book), so the loop stays bounded at self-oscillation.
 *  - Envelopes: StAdsr through st_env_* (studio_common.c). Glide and the
 *    pitch envelope are exponential ("RC") approaches that cover 99% of
 *    their way in glide_time / pitch_env_time, like the ADSR segments.
 *  - Modulation runs at the control rate (every ST_SYNTH_CTRL samples) and
 *    is ramped linearly per sample: phase increment, filter g, amplitude,
 *    pan and pulse width, so nothing zippers.
 *  - After the filter a soft knee (exact below 1.5, ceiling 2.5) keeps a
 *    pathological patch from reaching the mixer at +30 dB.
 *  - tanh: a rational approximation that reaches exactly +-1 at +-3 with
 *    zero slope, so it is smooth, cheap and identical on every platform.
 */
#include <math.h>
#include <stddef.h>
#include <string.h>

#include "studio_synth.h"

#define PI_D 3.14159265358979323846
#define PI_F 3.14159265f
#define SQRT2_F 1.41421356f

/* ln(100): an exponential segment covers 99% of its way in its time,
 * as in studio_common.c. */
#define LN100 4.605170f

/* Ladder: the saturator's headroom; a signal up to about this level goes
 * through the loop nearly clean. */
#define LADDER_HEADROOM 2.0f

/* SVF: where the band-pass integrator starts to saturate. */
#define SVF_BAND_KNEE 1.5f

/* The voice's safety limit after the filter: exact below OUT_KNEE, never
 * above OUT_CEIL (times at most sqrt(2) from a hard pan), so a stack of
 * seven at full resonance cannot reach the mixer at +30 dB. */
#define OUT_KNEE 1.5f
#define OUT_CEIL 2.5f

/* Values below this are flushed to zero once per control step, so a
 * decaying filter never runs into denormals. */
#define FLUSH_EPS 1e-15f

static float clampf(float x, float lo, float hi)
{
    if (!(x >= lo)) /* also catches NaN */
        return lo;
    return x > hi ? hi : x;
}

static float flush(float x)
{
    return fabsf(x) < FLUSH_EPS ? 0.0f : x;
}

/* tanh(x), rational: x(27 + x^2) / (27 + 9x^2) on [-3, 3], +-1 beyond.
 * Error under 0.025; continuous with zero slope at +-3. */
static float soft_tanh(float x)
{
    if (x >= 3.0f)
        return 1.0f;
    if (x <= -3.0f)
        return -1.0f;
    float x2 = x * x;
    return x * (27.0f + x2) / (27.0f + 9.0f * x2);
}

/* Transparent below t, then bends smoothly (slope 1 at t) toward a
 * ceiling h: a soft limit that leaves normal levels bit-exact. */
static float knee(float x, float t, float h)
{
    float a = fabsf(x);
    if (a <= t)
        return x;
    float w = h - t;
    float y = t + w * soft_tanh((a - t) / w);
    return x < 0.0f ? -y : y;
}

/* A seed spread over all 32 bits (the murmur3 finaliser), so nearby seeds
 * give unrelated xorshift streams; never zero. */
static uint32_t mix_seed(uint32_t s)
{
    s ^= s >> 16;
    s *= 0x85EBCA6Bu;
    s ^= s >> 13;
    s *= 0xC2B2AE35u;
    s ^= s >> 16;
    return s ? s : 0x9E3779B9u;
}

/* Uniform phase in [0, 1). */
static double rand_phase(uint32_t *rng)
{
    return (double)(st_rand(rng) >> 8) / 16777216.0;
}

/* ------------------------------------------------------------------ */
/* Oscillators.                                                        */

/* PolyBLEP residual for a unit-height (+2 peak to peak / 2) step at phase
 * 0: t is the phase, dt the increment, both in cycles. Correction for a
 * step of height h is (h / 2) * blep. */
static double blep(double t, double dt)
{
    if (t < dt) {
        t /= dt;
        return t + t - t * t - 1.0;
    }
    if (t > 1.0 - dt) {
        t = (t - 1.0) / dt;
        return t * t + t + t + 1.0;
    }
    return 0.0;
}

/* Four-sample PolyBLEP: the residual of a step band-limited by the
 * cubic B-spline (the integrated third-order B-spline of Valimaki, Pekonen
 * and Nam, "Perceptually informed synthesis of bandlimited classical
 * waveforms using integrated polynomial interpolation", JASA 2012),
 * spanning two samples on each side of the step. It suppresses aliasing
 * markedly better than the two-sample form above, at the cost of a
 * slightly softer top octave. Same convention: (h / 2) * blep4. A period
 * must be longer than four samples (dt < 0.25, a fundamental under 12 kHz
 * at 48 kHz); above that the two-sample form takes over. */
static double blep4(double t, double dt)
{
    if (dt >= 0.25)
        return blep(t, dt);
    if (t < 2.0 * dt) {           /* after the step, x = 0..2 samples */
        double x = t / dt;
        if (x < 1.0) {
            double x2 = x * x;
            return 2.0 * (-0.5 + x * (2.0 / 3.0) - x2 * x / 3.0 +
                          x2 * x2 / 8.0);
        }
        x = 2.0 - x;
        return -(x * x) * (x * x) / 12.0;
    }
    if (t > 1.0 - 2.0 * dt) {     /* before it, x = -2..0 samples */
        double x = (t - 1.0) / dt;
        if (x > -1.0) {
            double x2 = x * x;
            return 2.0 * (0.5 + x * (2.0 / 3.0) - x2 * x / 3.0 -
                          x2 * x2 / 8.0);
        }
        x = 2.0 + x;
        return (x * x) * (x * x) / 12.0;
    }
    return 0.0;
}

/* PolyBLAMP residual (the integral of the BLEP residual above, in
 * samples) for a slope change of 1 per sample at phase 0. Correction for
 * a slope change of s per sample is s * blamp. */
static double blamp(double t, double dt)
{
    if (t < dt) {
        t = 1.0 - t / dt;            /* (1 - x)^3 / 6 after the corner */
        return t * t * t / 6.0;
    }
    if (t > 1.0 - dt) {
        t = (t - 1.0) / dt + 1.0;    /* (x + 1)^3 / 6 before it */
        return t * t * t / 6.0;
    }
    return 0.0;
}

static double wrap1(double t)
{
    return t >= 1.0 ? t - 1.0 : t;
}

/* One sample of a band-limited waveform at phase t (cycles), increment dt
 * (cycles per sample), pulse width pw. */
static float osc_sample(StWave wave, double t, double dt, double pw)
{
    switch (wave) {
    case ST_WAVE_SAW:
        /* falling step of -2 at t = 0 */
        return (float)(2.0 * t - 1.0 - blep4(t, dt));
    case ST_WAVE_SQUARE:
    case ST_WAVE_PULSE: {
        /* +2 step at 0, -2 step at pw; the DC of an asymmetric pulse
         * (2pw - 1) is removed so width modulation does not thump. */
        double v = t < pw ? 1.0 : -1.0;
        v += blep4(t, dt) - blep4(wrap1(t + 1.0 - pw), dt);
        return (float)(v - (2.0 * pw - 1.0));
    }
    case ST_WAVE_TRIANGLE: {
        /* -1 at t = 0, +1 at t = 0.5: slope +-4 per cycle, so each corner
         * changes the slope by 8 dt per sample. */
        double v = 1.0 - 4.0 * fabs(t - 0.5);
        v += 8.0 * dt * (blamp(t, dt) - blamp(wrap1(t + 0.5), dt));
        return (float)v;
    }
    case ST_WAVE_SINE:
        return (float)sin(2.0 * PI_D * t);
    case ST_WAVE_NOISE: /* handled by the caller */
    default:
        return 0.0f;
    }
}

/* ------------------------------------------------------------------ */
/* Filters.                                                            */

/* Simper SVF, one sample. s[0] = ic1eq, s[1] = ic2eq; k = 1/Q. */
static float svf_tick(float *s, float x, float g, float k, StFilterMode mode)
{
    float a1 = 1.0f / (1.0f + g * (g + k));
    float a2 = g * a1;
    float a3 = g * a2;
    float v3 = x - s[1];
    float v1 = a1 * s[0] + a2 * v3;
    float v2 = s[1] + a2 * s[0] + a3 * v3;
    s[0] = 2.0f * v1 - s[0];
    s[1] = 2.0f * v2 - s[1];
    /* For LP and HP the band state soft-limits past SVF_BAND_KNEE, as an
     * analog SVF's integrators saturate: a resonance fed at its own
     * frequency (gain Q, up to ~28) rings loud but bounded instead of 30x
     * the input. Below the knee the filter is exactly linear. The
     * normalised band-pass (k v1) never exceeds its input, so it is left
     * linear. */
    if (mode != ST_FILTER_BP)
        s[0] = knee(s[0], SVF_BAND_KNEE, 2.0f * SVF_BAND_KNEE);
    switch (mode) {
    case ST_FILTER_HP:
        return x - k * v1 - v2;
    case ST_FILTER_BP:
        return k * v1; /* constant 0 dB peak gain */
    case ST_FILTER_LP:
    default:
        return v2;
    }
}

/* Zavalishin TPT ladder, one sample. s[0..3] are the one-poles' states,
 * k = feedback 0..~4, comp = output make-up. */
static float ladder_tick(float *s, float x, float g, float k, float comp)
{
    float b = 1.0f / (1.0f + g);     /* the state's weight in a stage */
    float G = g * b;                 /* a stage's instantaneous gain */
    /* y4 = G^4 u + S, with S the states' contribution; solving
     * u = x - k y4 for the linear loop gives y4, then the junction
     * saturates. */
    float S = b * (G * (G * (G * s[0] + s[1]) + s[2]) + s[3]);
    float G4 = (G * G) * (G * G);
    float y4 = (G4 * x + S) / (1.0f + k * G4);
    float u = LADDER_HEADROOM * soft_tanh((x - k * y4) / LADDER_HEADROOM);
    for (int i = 0; i < 4; i++) {
        float vv = (u - s[i]) * G;
        float y = vv + s[i];
        s[i] = y + vv;
        u = y;
    }
    return u * comp;
}

/* ------------------------------------------------------------------ */
/* LFOs.                                                               */

/* Advance LFO i by dt seconds and return its value, -1..1, scaled by its
 * delay fade-in. */
static float lfo_step(StSynthVoice *v, int i, float dt)
{
    const StLfo *lfo = &v->patch->lfo[i];
    float rate = clampf(lfo->rate_hz, 0.0f, 200.0f);
    v->lfo_phase[i] += rate * dt;
    if (v->lfo_phase[i] >= 1.0f) {
        v->lfo_phase[i] -= floorf(v->lfo_phase[i]);
        v->lfo_hold[i] = st_randf(&v->rng); /* a new step each cycle */
    }
    float ph = v->lfo_phase[i];
    float val;
    switch (lfo->shape) {
    case ST_LFO_TRIANGLE:
        val = 1.0f - 4.0f * fabsf(ph - 0.5f);
        break;
    case ST_LFO_SQUARE:
        val = ph < 0.5f ? 1.0f : -1.0f;
        break;
    case ST_LFO_SAMPLE_HOLD:
        val = v->lfo_hold[i];
        break;
    case ST_LFO_SINE:
    default:
        val = sinf(2.0f * PI_F * ph);
        break;
    }
    /* Silent for `delay` seconds, then fades in over the same time. */
    if (lfo->delay > 0.0f)
        val *= clampf((v->lfo_time - lfo->delay) / lfo->delay, 0.0f, 1.0f);
    return val;
}

/* ------------------------------------------------------------------ */
/* Control rate.                                                       */

/* Equal-power pan gains normalised to unity at centre: pan -1..1. At
 * pan 0 both are exactly 1, so a centred voice is bit-identical L/R. */
static void pan_gains(float pan, float *gl, float *gr)
{
    if (pan == 0.0f) {
        *gl = *gr = 1.0f;
        return;
    }
    pan = clampf(pan, -1.0f, 1.0f);
    *gl = SQRT2_F * cosf((pan + 1.0f) * (PI_F * 0.25f));
    *gr = SQRT2_F * cosf((1.0f - pan) * (PI_F * 0.25f));
}

/* Exponential approach covering 99% of the way in `secs`, per step of
 * `step` seconds. */
static float approach_coeff(float secs, float step)
{
    if (secs <= 0.0f)
        return 1.0f;
    return 1.0f - expf(-LN100 * step / secs);
}

/* Compute the modulation targets for the end of the next control step
 * and the per-sample ramps toward them. */
static void control_step(StSynthVoice *v, const StVoiceMod *mod)
{
    const StSynthPatch *p = v->patch;
    float ctrl_rate = v->rate / (float)ST_SYNTH_CTRL;
    float dt = 1.0f / ctrl_rate;

    /* Glide and pitch envelope (both exponential, "RC"-like). */
    v->note_cur += (v->note_target - v->note_cur) *
                   approach_coeff(p->glide_time, dt);
    if (fabsf(v->note_target - v->note_cur) < 1e-4f)
        v->note_cur = v->note_target;
    v->pitch_env -= v->pitch_env * approach_coeff(p->pitch_env_time, dt);
    if (fabsf(v->pitch_env) < 1e-5f)
        v->pitch_env = 0.0f;

    /* Filter envelope, stepped at the control rate. */
    float fenv = st_env_next(&v->filt_env, &p->filter_env, ctrl_rate);

    /* LFOs. */
    float pitch_c = mod ? mod->pitch_cents : 0.0f;
    float cut_oct = mod ? mod->cutoff_oct : 0.0f;
    float amp = clampf(v->velocity, 0.0f, 1.0f);
    float pan = 0.0f;
    float pw = p->wave == ST_WAVE_PULSE ? p->pulse_width : 0.5f;
    v->lfo_time += dt;
    for (int i = 0; i < 2; i++) {
        const StLfo *lfo = &p->lfo[i];
        float val = lfo_step(v, i, dt); /* free-running, routed or not */
        pitch_c += val * lfo->to_pitch_cents;
        cut_oct += val * lfo->to_cutoff_oct;
        amp *= 1.0f - clampf(lfo->to_amp, 0.0f, 1.0f) * (0.5f + 0.5f * val);
        pan += val * clampf(lfo->to_pan, 0.0f, 1.0f);
        pw += val * clampf(lfo->to_pulse_width, 0.0f, 0.45f);
    }
    pw = clampf(pw, 0.05f, 0.95f);

    /* Pitch -> phase increment, kept below the PolyBLEP's limit. */
    float pitch = v->note_cur + v->pitch_env + pitch_c * 0.01f;
    pitch = clampf(pitch, -60.0f, 200.0f);
    double inc = 440.0 * exp2(((double)pitch - 69.0) / 12.0) / (double)v->rate;
    if (inc > 0.45)
        inc = 0.45;

    /* Cutoff in octaves around cutoff_hz, clamped to [20, 0.45 fs]. */
    float g = 0.0f;
    if (p->filter != ST_FILTER_NONE) {
        float oct = p->key_track * (v->note_cur - 60.0f) / 12.0f +
                    p->env_amount_oct * fenv +
                    p->velocity_to_cutoff_oct * v->velocity + cut_oct;
        oct = clampf(oct, -20.0f, 20.0f);
        float base = p->cutoff_hz > 20.0f ? p->cutoff_hz : 20.0f;
        float fc = clampf(base * exp2f(oct), 20.0f, 0.45f * v->rate);
        g = tanf(PI_F * fc / v->rate);
    }

    float pl, pr;
    pan_gains(clampf(pan, -1.0f, 1.0f), &pl, &pr);

    float inv = 1.0f / (float)ST_SYNTH_CTRL;
    if (!v->primed) {
        v->inc = inc;
        v->g = g;
        v->amp = amp;
        v->pan_l = pl;
        v->pan_r = pr;
        v->pw = pw;
        v->inc_step = 0.0;
        v->g_step = v->amp_step = v->pan_l_step = v->pan_r_step = 0.0f;
        v->pw_step = 0.0f;
        v->primed = 1;
    } else {
        v->inc_step = (inc - v->inc) / ST_SYNTH_CTRL;
        v->g_step = (g - v->g) * inv;
        v->amp_step = (amp - v->amp) * inv;
        v->pan_l_step = (pl - v->pan_l) * inv;
        v->pan_r_step = (pr - v->pan_r) * inv;
        v->pw_step = (pw - v->pw) * inv;
    }

    for (int c = 0; c < 2; c++)
        for (int j = 0; j < 4; j++)
            v->flt[c][j] = flush(v->flt[c][j]);
}

/* ------------------------------------------------------------------ */
/* API.                                                                */

void st_synth_start(StSynthVoice *v, const StSynthPatch *p, float rate,
                    float note, float velocity, uint32_t seed)
{
    memset(v, 0, sizeof *v);
    v->patch = p;
    v->rate = rate > 1000.0f ? rate : (float)ST_DEFAULT_RATE;
    v->active = 1;
    v->rng = mix_seed(seed);
    v->velocity = clampf(velocity, 0.0f, 1.0f);
    v->note_target = v->note_cur = note;
    v->pitch_env = p->pitch_env_time > 0.0f ? p->pitch_env_semis : 0.0f;

    /* The stack: copies detuned symmetrically (the outermost at
     * +-detune/2), panned evenly across +-stereo_spread, each with a
     * random start phase; level 1/sqrt(N) keeps the summed power of N
     * uncorrelated copies at that of one. */
    int n = p->stack < 1 ? 1 : (p->stack > ST_SYNTH_MAX_STACK
                                    ? ST_SYNTH_MAX_STACK : p->stack);
    if (p->wave == ST_WAVE_NOISE)
        n = 1;
    v->stack = n;
    float spread = clampf(p->stereo_spread, 0.0f, 1.0f);
    v->stereo = n > 1 && spread > 0.0f;
    float norm = 1.0f / sqrtf((float)n);
    for (int k = 0; k < n; k++) {
        float pos = n > 1 ? (float)k / (float)(n - 1) - 0.5f : 0.0f;
        float det = clampf(p->detune_cents, -1200.0f, 1200.0f);
        v->ratio[k] = exp2((double)(det * pos) / 1200.0);
        v->phase[k] = rand_phase(&v->rng);
        float gl = 1.0f, gr = 1.0f;
        if (v->stereo)
            pan_gains(2.0f * pos * spread, &gl, &gr);
        v->gain_l[k] = gl * norm;
        v->gain_r[k] = gr * norm;
    }
    v->sub_phase = rand_phase(&v->rng);
    for (int i = 0; i < 2; i++) {
        v->lfo_phase[i] = (float)rand_phase(&v->rng);
        v->lfo_hold[i] = st_randf(&v->rng);
    }

    st_env_on(&v->amp_env);
    st_env_on(&v->filt_env);
    v->primed = 0;
    v->ctrl_left = 0;
}

void st_synth_legato(StSynthVoice *v, float note, float velocity)
{
    if (!v->active)
        return;
    v->note_target = note;
    if (v->patch->glide_time <= 0.0f)
        v->note_cur = note;
    v->velocity = clampf(velocity, 0.0f, 1.0f);
    if (v->amp_env.stage == 4) {
        st_env_on(&v->amp_env);  /* attack from the current level */
        st_env_on(&v->filt_env);
    }
}

void st_synth_release(StSynthVoice *v)
{
    st_env_off(&v->amp_env);
    st_env_off(&v->filt_env);
}

int st_synth_active(const StSynthVoice *v)
{
    return v->active;
}

int st_synth_render(StSynthVoice *v, const StVoiceMod *mod, float *l,
                    float *r, int n)
{
    if (!v->active)
        return 0;
    const StSynthPatch *p = v->patch;
    const StWave wave = p->wave;
    const int stack = v->stack;
    const int stereo = v->stereo;
    const StFilterMode fmode = p->filter;
    const float sub_level = clampf(p->sub_level, 0.0f, 1.0f);
    const float noise_level = clampf(p->noise_level, 0.0f, 1.0f);

    /* Drive: tanh(g x) / tanh(g) with g = 1..8, so a full-scale input
     * stays at full scale while its shape squares off; cross-faded in
     * over the first quarter of the range so small drive is subtle. */
    const float drive = clampf(p->drive, 0.0f, 1.0f);
    const float dgain = 1.0f + 7.0f * drive;
    const float dnorm = 1.0f / soft_tanh(dgain);
    const float dmix = clampf(drive * 4.0f, 0.0f, 1.0f);

    /* Resonance. SVF: k = 1/Q from sqrt(2) (Butterworth, Q 0.707) at 0
     * down to 0.035 (Q ~ 28) at 1, exponentially. Ladder: feedback up to
     * 3.96, just short of the linear self-oscillation point 4, with half
     * the passband loss (1 + k)^-1 made up. */
    const float res = clampf(p->resonance, 0.0f, 1.0f);
    const float k_svf = SQRT2_F * powf(0.025f, res);
    const float k_lad = 3.96f * res;
    const float comp = 1.0f + 0.5f * k_lad;

    float g0 = mod ? mod->gain0 : 1.0f;
    float g1 = mod ? mod->gain1 : 1.0f;
    float gstep = n > 0 ? (g1 - g0) / (float)n : 0.0f;

    for (int i = 0; i < n; i++) {
        if (v->ctrl_left == 0) {
            control_step(v, mod);
            v->ctrl_left = ST_SYNTH_CTRL;
        }
        v->ctrl_left--;
        v->inc += v->inc_step;
        v->g += v->g_step;
        v->amp += v->amp_step;
        v->pan_l += v->pan_l_step;
        v->pan_r += v->pan_r_step;
        v->pw += v->pw_step;

        const double inc = v->inc;
        const double pw = v->pw;

        /* Oscillators. */
        float xl = 0.0f, xr = 0.0f;
        if (wave == ST_WAVE_NOISE) {
            xl = xr = st_randf(&v->rng);
        } else {
            for (int k = 0; k < stack; k++) {
                double dt = inc * v->ratio[k];
                if (dt > 0.49) /* a period must span two samples */
                    dt = 0.49;
                float s = osc_sample(wave, v->phase[k], dt, pw);
                xl += s * v->gain_l[k];
                xr += s * v->gain_r[k];
                v->phase[k] = wrap1(v->phase[k] + dt);
            }
        }
        float common = 0.0f;
        if (sub_level > 0.0f) {
            common += sub_level * (float)sin(2.0 * PI_D * v->sub_phase);
        }
        v->sub_phase = wrap1(v->sub_phase + 0.5 * inc);
        if (noise_level > 0.0f)
            common += noise_level * st_randf(&v->rng);
        xl += common;
        xr += common;

        /* Drive. */
        if (dmix > 0.0f) {
            xl += dmix * (soft_tanh(dgain * xl) * dnorm - xl);
            if (stereo)
                xr += dmix * (soft_tanh(dgain * xr) * dnorm - xr);
        }

        /* Filter (once when mono, so L == R exactly). */
        float g = v->g;
        switch (fmode) {
        case ST_FILTER_LP:
        case ST_FILTER_HP:
        case ST_FILTER_BP:
            xl = svf_tick(v->flt[0], xl, g, k_svf, fmode);
            if (stereo)
                xr = svf_tick(v->flt[1], xr, g, k_svf, fmode);
            break;
        case ST_FILTER_LADDER:
            xl = ladder_tick(v->flt[0], xl, g, k_lad, comp);
            if (stereo)
                xr = ladder_tick(v->flt[1], xr, g, k_lad, comp);
            break;
        case ST_FILTER_NONE:
        default:
            break;
        }
        xl = knee(xl, OUT_KNEE, OUT_CEIL);
        if (stereo)
            xr = knee(xr, OUT_KNEE, OUT_CEIL);
        else
            xr = xl;

        /* Amplitude and pan. */
        float env = st_env_next(&v->amp_env, &p->amp, v->rate);
        float gain = env * v->amp * (g0 + gstep * (float)i);
        l[i] += xl * gain * v->pan_l;
        r[i] += xr * gain * v->pan_r;

        if (v->amp_env.stage == 0) {
            v->active = 0;
            return 0;
        }
    }
    return 1;
}
