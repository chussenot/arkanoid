/*
 * studio_fx.c: the studio's effects (BRYX addition; see studio.h and
 * studio_fx.h). Reverb, delay, biquads, compressor, limiter, all float
 * stereo, all parameterised in Hz / seconds / dB.
 */
#include "studio_fx.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

#define ST_PI 3.14159265358979323846

/* Flushes values too small to matter to zero. Adding then subtracting a
 * constant is not an identity in IEEE arithmetic (it is kept without
 * -ffast-math), so anything below ~1e-25 becomes exactly 0: a decaying
 * tail never reaches the denormal range, where x86 slows down by ~100x. */
static float flush_tiny(float x)
{
    x += 1e-18f;
    x -= 1e-18f;
    return x;
}

static float clampf(float x, float lo, float hi)
{
    return x < lo ? lo : (x > hi ? hi : x);
}

/* ================================================================== */
/* Biquads: RBJ Audio EQ Cookbook (Robert Bristow-Johnson, 2005;      */
/* W3C Audio EQ Cookbook note, 2021).                                 */

static double bq_omega(float rate, float hz)
{
    double f = hz;
    double nyq = 0.49 * (double)rate;
    if (f < 1.0) f = 1.0;
    if (f > nyq) f = nyq;
    return 2.0 * ST_PI * f / (double)rate;
}

static void bq_set(StBiquad *b, double b0, double b1, double b2, double a0,
                   double a1, double a2)
{
    b->b0 = b0 / a0;
    b->b1 = b1 / a0;
    b->b2 = b2 / a0;
    b->a1 = a1 / a0;
    b->a2 = a2 / a0;
}

void st_biquad_lowpass(StBiquad *b, float rate, float hz, float q)
{
    double w = bq_omega(rate, hz), cw = cos(w);
    double alpha = sin(w) / (2.0 * (q > 0.01f ? q : 0.01f));
    bq_set(b, (1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0,
           1.0 + alpha, -2.0 * cw, 1.0 - alpha);
}

void st_biquad_highpass(StBiquad *b, float rate, float hz, float q)
{
    double w = bq_omega(rate, hz), cw = cos(w);
    double alpha = sin(w) / (2.0 * (q > 0.01f ? q : 0.01f));
    bq_set(b, (1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0,
           1.0 + alpha, -2.0 * cw, 1.0 - alpha);
}

void st_biquad_peak(StBiquad *b, float rate, float hz, float q, float gain_db)
{
    double A = pow(10.0, gain_db / 40.0);
    double w = bq_omega(rate, hz), cw = cos(w);
    double alpha = sin(w) / (2.0 * (q > 0.01f ? q : 0.01f));
    bq_set(b, 1.0 + alpha * A, -2.0 * cw, 1.0 - alpha * A,
           1.0 + alpha / A, -2.0 * cw, 1.0 - alpha / A);
}

/* Shelf slope S = 1: alpha = sin(w)/2 * sqrt((A + 1/A)(1/S - 1) + 2). */
void st_biquad_lowshelf(StBiquad *b, float rate, float hz, float gain_db)
{
    double A = pow(10.0, gain_db / 40.0);
    double w = bq_omega(rate, hz), cw = cos(w);
    double alpha = sin(w) / 2.0 * sqrt(2.0);
    double sa = 2.0 * sqrt(A) * alpha;
    bq_set(b, A * ((A + 1.0) - (A - 1.0) * cw + sa),
           2.0 * A * ((A - 1.0) - (A + 1.0) * cw),
           A * ((A + 1.0) - (A - 1.0) * cw - sa),
           (A + 1.0) + (A - 1.0) * cw + sa,
           -2.0 * ((A - 1.0) + (A + 1.0) * cw),
           (A + 1.0) + (A - 1.0) * cw - sa);
}

void st_biquad_highshelf(StBiquad *b, float rate, float hz, float gain_db)
{
    double A = pow(10.0, gain_db / 40.0);
    double w = bq_omega(rate, hz), cw = cos(w);
    double alpha = sin(w) / 2.0 * sqrt(2.0);
    double sa = 2.0 * sqrt(A) * alpha;
    bq_set(b, A * ((A + 1.0) + (A - 1.0) * cw + sa),
           -2.0 * A * ((A - 1.0) + (A + 1.0) * cw),
           A * ((A + 1.0) + (A - 1.0) * cw - sa),
           (A + 1.0) - (A - 1.0) * cw + sa,
           2.0 * ((A - 1.0) - (A + 1.0) * cw),
           (A + 1.0) - (A - 1.0) * cw - sa);
}

void st_biquad_identity(StBiquad *b)
{
    bq_set(b, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
}

void st_biquad_reset(StBiquad *b)
{
    b->z1[0] = b->z1[1] = 0.0;
    b->z2[0] = b->z2[1] = 0.0;
}

/* One sample through channel ch (transposed direct form II). */
static float bq_tick(StBiquad *b, int ch, float x)
{
    double y = b->b0 * x + b->z1[ch];
    b->z1[ch] = b->b1 * x - b->a1 * y + b->z2[ch];
    b->z2[ch] = b->b2 * x - b->a2 * y;
    return (float)y;
}

void st_biquad_process(StBiquad *b, float *l, float *r, int n)
{
    for (int i = 0; i < n; i++)
        l[i] = bq_tick(b, 0, l[i]);
    if (r)
        for (int i = 0; i < n; i++)
            r[i] = bq_tick(b, 1, r[i]);
}

/* ================================================================== */
/* Reverb.                                                            */
/*
 * Design: a feedback delay network (FDN), the structure introduced by
 * Stautner & Puckette ("Designing multi-channel reverberators", Computer
 * Music Journal 6(1), 1982) and made tunable by Jot & Chaigne ("Digital
 * delay networks for designing artificial reverberators", AES 90th
 * Convention, 1991). See also J. O. Smith, "Physical Audio Signal
 * Processing", ch. "Feedback Delay Networks", and Valimaki et al., "Fifty
 * years of artificial reverberation", IEEE TASLP 20(5), 2012.
 *
 *   in L/R -> high-pass (low_cut_hz) -> pre-delay line --+--> early
 *                                                        |    reflections
 *                                                        v    (taps)
 *     +--> [line_i: z^-len_i] -> g_i * one-pole LP_i -> s_i --> c_L, c_R
 *     |                                                   |
 *     +--- input_i + (A s)_i <--- Householder A <---------+
 *
 * - Eight delay lines, lengths a geometric series (ratio ~1.26) from
 *   22 ms to 113 ms at size 1, each rounded up to a prime number of
 *   frames so no two share a factor (their echoes never line up), and
 *   scaled by 0.25 + 0.75 * size. All in seconds, so the network is the
 *   same at 44.1 and 48 kHz to within a frame per line.
 * - Feedback matrix: the Householder reflection A = I - (2/N) 1 1^T. It
 *   is orthogonal (lossless: the loop by itself never gains or loses
 *   energy) and maximally diffusing for N = 8, and costs one sum per
 *   sample instead of a matrix multiply (Jot 1991; Smith, PASP).
 * - Decay: because A is lossless, all loss is in the per-line gains.
 *   Jot's rule makes every line lose the same dB per second:
 *       g_i = 10^(-3 len_i / (rt60 * rate))
 *   so the tail is a clean exponential reaching -60 dB at rt60.
 * - Frequency-dependent decay: each line's gain is a one-pole low-pass
 *   g_i (1 - a_i) / (1 - a_i z^-1): unity (times g_i) at DC, and a_i is
 *   solved per line so its magnitude at damping_hz is g_i once more,
 *   i.e. the reverberation time at damping_hz is rt60 / 2 for every
 *   line (Jot's "absorbent filters"). Above it the tail dies faster
 *   still, as in air and real rooms.
 * - Modulation: the four longest lines' read points move by up to
 *   0.6 ms * mod_depth under slow sine LFOs at unrelated rates (0.31 to
 *   0.89 Hz, at most a few cents of pitch), read with linear
 *   interpolation. This smears the network's fixed modes so a long tail
 *   does not ring metallically.
 * - Stereo: even lines take the left input, odd lines the right; the two
 *   outputs read the lines through two orthogonal sign patterns (Hadamard
 *   rows, both orthogonal to the all-ones vector A treats specially), so
 *   the late tails are decorrelated. Width is a mid/side scale on the
 *   return.
 * - Early reflections: six taps per side off the pre-delay line, the
 *   first exactly at the pre-delay, alternating sources across sides.
 *   They give the space its first arrival and a sense of size before the
 *   network's echo density builds up.
 * - Level: the late-tail output gain is set from the network's energy
 *   gain, sum_i 1 / (1 - g_i^2), so every space returns about the same
 *   energy for a given send (about -6 dB for an impulse, within 2 dB
 *   across the presets): changing space changes the character, not the
 *   balance.
 */

/* Line lengths at size 1, milliseconds. */
static const float fdn_ms[ST_FDN_LINES] = {
    22.1f, 28.3f, 35.9f, 45.1f, 56.9f, 71.3f, 89.9f, 113.3f
};
/* Output sign patterns (Hadamard rows 1 and 2). */
static const float fdn_out_l[ST_FDN_LINES] = { 1, -1, 1, -1, 1, -1, 1, -1 };
static const float fdn_out_r[ST_FDN_LINES] = { 1, 1, -1, -1, 1, 1, -1, -1 };
/* Modulation LFO rates (Hz) for lines 4..7. */
static const double fdn_lfo_hz[ST_FDN_LFOS] = { 0.31, 0.47, 0.67, 0.89 };
#define FDN_FIRST_MOD_LINE (ST_FDN_LINES - ST_FDN_LFOS)
#define FDN_MOD_MAX_S 0.0006f
/* Early reflections at size 1, ms after the pre-delay; tap k on the left
 * reads the left input when k is even and the right when odd, and the
 * right side mirrors it. */
static const float er_ms_l[ST_ER_TAPS] = { 0.0f, 4.3f, 9.7f, 15.1f, 22.9f, 31.3f };
static const float er_ms_r[ST_ER_TAPS] = { 0.0f, 6.1f, 11.3f, 18.7f, 25.3f, 34.1f };
static const float er_rel[ST_ER_TAPS] = { 1.0f, -0.8f, 0.62f, -0.5f, 0.38f, -0.3f };
/* Energy gain of the late tail, before the damping filters take their
 * share: measured IR energy per channel, early reflections included, is
 * about -6 dB (-5 to -7 across the presets) for a unit impulse on both
 * inputs. */
#define FDN_TARGET_ENERGY 1.0

static int is_prime(int n)
{
    if (n < 2) return 0;
    if (n % 2 == 0) return n == 2;
    for (int d = 3; d * d <= n; d += 2)
        if (n % d == 0) return 0;
    return 1;
}

static int next_prime(int n)
{
    while (!is_prime(n)) n++;
    return n;
}

int st_reverb_preset(const char *name, StReverbParams *out)
{
    /* rt60, predelay, size, damping_hz, low_cut_hz, width, mod_depth.
     * The four spaces of docs/design/orchestral-score.md section 4.2:
     * room (tight, for percussion and synths), stage (the orchestra's
     * own space), hall (the big cinematic return), long (transitions
     * and stingers). RT60s are checked by tests/test_fx.c. */
    static const struct { const char *name; StReverbParams p; } presets[] = {
        { "room",  { 0.6f, 0.005f, 0.30f, 7000.0f, 120.0f, 0.8f, 0.20f } },
        { "stage", { 1.2f, 0.012f, 0.55f, 6000.0f, 100.0f, 0.9f, 0.30f } },
        { "hall",  { 2.4f, 0.025f, 0.80f, 5000.0f,  80.0f, 1.0f, 0.40f } },
        { "long",  { 5.5f, 0.040f, 1.00f, 4000.0f,  60.0f, 1.0f, 0.50f } },
    };
    if (!name || !out) return 0;
    for (size_t i = 0; i < sizeof presets / sizeof presets[0]; i++) {
        if (strcmp(name, presets[i].name) == 0) {
            *out = presets[i].p;
            return 1;
        }
    }
    return 0;
}

/* The pole a of g(1-a)/(1-a z^-1) whose magnitude (before g) is m at
 * angular frequency w: (1-a)^2 = m^2 (1 - 2a cos w + a^2), the smaller
 * root. m >= 1 or w out of range -> 0 (no damping). */
static float damping_pole(double m, double w)
{
    double M, A, B;
    if (m >= 1.0 || w <= 0.0 || w >= ST_PI) return 0.0f;
    M = m * m;
    A = 1.0 - M;
    B = 1.0 - M * cos(w);
    return (float)((B - sqrt(B * B - A * A)) / A);
}

void st_reverb_free(StReverb *r)
{
    if (!r) return;
    free(r->mem);
    memset(r, 0, sizeof(*r));
}

int st_reverb_init(StReverb *r, float rate, const StReverbParams *pin)
{
    StReverbParams p;
    float scale;
    double energy = 0.0;
    size_t total;
    float *m;
    int er_max = 0;

    if (!r) return -1;
    memset(r, 0, sizeof(*r));
    if (!pin || !(rate >= 8000.0f)) return -1;
    p = *pin;
    p.rt60 = clampf(p.rt60, 0.05f, 30.0f);
    p.predelay = clampf(p.predelay, 0.0f, 0.5f);
    p.size = clampf(p.size, 0.0f, 1.0f);
    p.width = clampf(p.width, 0.0f, 1.0f);
    p.mod_depth = clampf(p.mod_depth, 0.0f, 1.0f);
    r->rate = rate;
    r->p = p;
    r->width = p.width;
    scale = 0.25f + 0.75f * p.size;

    /* Input high-pass. */
    if (p.low_cut_hz > 0.0f)
        st_biquad_highpass(&r->low_cut, rate, p.low_cut_hz, 0.7071f);
    else
        st_biquad_identity(&r->low_cut);
    st_biquad_reset(&r->low_cut);

    /* Pre-delay and early reflections. */
    r->pre_delay = (int)lroundf(p.predelay * rate);
    for (int k = 0; k < ST_ER_TAPS; k++) {
        r->er_tap_l[k] = (int)lroundf(er_ms_l[k] * 0.001f * scale * rate);
        r->er_tap_r[k] = (int)lroundf(er_ms_r[k] * 0.001f * scale * rate);
        if (r->er_tap_l[k] > er_max) er_max = r->er_tap_l[k];
        if (r->er_tap_r[k] > er_max) er_max = r->er_tap_r[k];
        /* Smaller spaces have relatively stronger early reflections. */
        r->er_gain[k] = er_rel[k] * (0.32f - 0.16f * p.size);
    }
    r->pre_cap = r->pre_delay + er_max + 1;

    /* Lines. */
    r->mod_samples = p.mod_depth * FDN_MOD_MAX_S * rate;
    total = 2 * (size_t)r->pre_cap;
    for (int i = 0; i < ST_FDN_LINES; i++) {
        double len_s, w_d;
        int len = next_prime((int)lroundf(fdn_ms[i] * 0.001f * scale * rate));
        r->line_len[i] = len;
        /* room for the modulation excursion and the interpolation tap */
        r->line_cap[i] = len + (int)ceilf(r->mod_samples) + 4;
        total += (size_t)r->line_cap[i];

        len_s = (double)len / rate;
        r->fb_gain[i] = (float)pow(10.0, -3.0 * len_s / p.rt60);
        w_d = 2.0 * ST_PI * p.damping_hz / rate;
        r->damp_a[i] = (p.damping_hz > 0.0f && p.damping_hz < 0.45f * rate)
                           ? damping_pole(r->fb_gain[i], w_d) : 0.0f;
        r->damp_b[i] = r->fb_gain[i] * (1.0f - r->damp_a[i]);
        energy += 1.0 / (1.0 - (double)r->fb_gain[i] * r->fb_gain[i]);
    }
    r->out_gain = (float)sqrt(FDN_TARGET_ENERGY / energy);

    for (int k = 0; k < ST_FDN_LFOS; k++) {
        double w = 2.0 * ST_PI * fdn_lfo_hz[k] / rate;
        /* spread the starting phases so the excursions never align */
        double ph = 2.0 * ST_PI * (double)k / ST_FDN_LFOS;
        r->lfo_c[k] = cos(ph);
        r->lfo_s[k] = sin(ph);
        r->lfo_rc[k] = cos(w);
        r->lfo_rs[k] = sin(w);
    }

    m = calloc(total, sizeof(float));
    if (!m) {
        memset(r, 0, sizeof(*r));
        return -1;
    }
    r->mem = m;
    r->pre_l = m;
    m += r->pre_cap;
    r->pre_r = m;
    m += r->pre_cap;
    for (int i = 0; i < ST_FDN_LINES; i++) {
        r->line[i] = m;
        m += r->line_cap[i];
    }
    return 0;
}

/* Reading `k` frames back in a ring whose next write is at w. */
static int ring_back(int w, int k, int cap)
{
    int i = w - k;
    while (i < 0) i += cap;
    return i;
}

void st_reverb_process(StReverb *r, const float *in_l, const float *in_r,
                       float *out_l, float *out_r, int n)
{
    const float house = 2.0f / ST_FDN_LINES;
    const float side = 0.5f * r->width;

    for (int t = 0; t < n; t++) {
        float xl = bq_tick(&r->low_cut, 0, in_l[t]);
        float xr = bq_tick(&r->low_cut, 1, in_r[t]);
        float s[ST_FDN_LINES];
        float dl, dr, erl = 0.0f, err = 0.0f, yl = 0.0f, yr = 0.0f, sum = 0.0f;
        float wl, wr, mid, sid;

        /* pre-delay line: write, then read the pre-delayed input and the
         * early-reflection taps */
        r->pre_l[r->pre_w] = xl;
        r->pre_r[r->pre_w] = xr;
        dl = r->pre_l[ring_back(r->pre_w, r->pre_delay, r->pre_cap)];
        dr = r->pre_r[ring_back(r->pre_w, r->pre_delay, r->pre_cap)];
        for (int k = 0; k < ST_ER_TAPS; k++) {
            int il = ring_back(r->pre_w, r->pre_delay + r->er_tap_l[k], r->pre_cap);
            int ir = ring_back(r->pre_w, r->pre_delay + r->er_tap_r[k], r->pre_cap);
            erl += r->er_gain[k] * ((k & 1) ? r->pre_r[il] : r->pre_l[il]);
            err += r->er_gain[k] * ((k & 1) ? r->pre_l[ir] : r->pre_r[ir]);
        }
        if (++r->pre_w >= r->pre_cap) r->pre_w = 0;

        /* advance the LFO phasors (rotation, then a first-order
         * renormalisation so the amplitude cannot drift) */
        for (int k = 0; k < ST_FDN_LFOS; k++) {
            double c = r->lfo_c[k] * r->lfo_rc[k] - r->lfo_s[k] * r->lfo_rs[k];
            double sn = r->lfo_s[k] * r->lfo_rc[k] + r->lfo_c[k] * r->lfo_rs[k];
            double g = 1.5 - 0.5 * (c * c + sn * sn);
            r->lfo_c[k] = c * g;
            r->lfo_s[k] = sn * g;
        }

        /* read every line, through its damping filter */
        for (int i = 0; i < ST_FDN_LINES; i++) {
            float *buf = r->line[i];
            int cap = r->line_cap[i], w = r->line_w[i];
            float o;
            if (i >= FDN_FIRST_MOD_LINE && r->mod_samples > 0.0f) {
                float d = (float)r->line_len[i] +
                          r->mod_samples * (float)r->lfo_s[i - FDN_FIRST_MOD_LINE];
                int k = (int)d;
                float f = d - (float)k;
                o = (1.0f - f) * buf[ring_back(w, k, cap)] +
                    f * buf[ring_back(w, k + 1, cap)];
            } else {
                o = buf[ring_back(w, r->line_len[i], cap)];
            }
            r->damp_z[i] = flush_tiny(r->damp_b[i] * o + r->damp_a[i] * r->damp_z[i]);
            s[i] = r->damp_z[i];
            yl += fdn_out_l[i] * s[i];
            yr += fdn_out_r[i] * s[i];
            sum += s[i];
        }

        /* Householder feedback plus the input, written back */
        sum *= house;
        for (int i = 0; i < ST_FDN_LINES; i++) {
            float v = s[i] - sum + ((i & 1) ? dr : dl);
            r->line[i][r->line_w[i]] = v;
            if (++r->line_w[i] >= r->line_cap[i]) r->line_w[i] = 0;
        }

        wl = erl + r->out_gain * yl;
        wr = err + r->out_gain * yr;
        mid = 0.5f * (wl + wr);
        sid = side * (wl - wr);
        out_l[t] += mid + sid;
        out_r[t] += mid - sid;
    }
}

/* ================================================================== */
/* Delay.                                                             */
/*
 * Two delay lines with fractional read points (4-point, 3rd-order
 * Hermite interpolation), each loop through a high-pass and a low-pass
 * biquad (Q 0.7071) so every repeat is a little thinner and darker than
 * the one before, like a tape echo:
 *
 *   stereo:     line_L <- in_L + fb * line_L;   line_R likewise
 *   ping-pong:  line_L <- (in_L + in_R)/2 + fb * line_R
 *               line_R <- fb * line_L
 *   (the write is filtered; the outputs are the lines' read points)
 *
 * So in ping-pong the first repeat is on the left at time_l, the second
 * on the right time_r later, the third back on the left, and so on.
 * Filters at 0 Hz (low cut) or at/above 0.45 * rate (high cut) are
 * bypassed exactly.
 */

#define DELAY_MIN_FRAMES 3.0 /* the Hermite read needs 2 frames ahead */

static void delay_filters(StDelay *d)
{
    if (d->p.low_cut_hz > 0.0f && d->p.low_cut_hz < 0.45f * d->rate)
        st_biquad_highpass(&d->low_cut, d->rate, d->p.low_cut_hz, 0.7071f);
    else
        st_biquad_identity(&d->low_cut);
    if (d->p.high_cut_hz > 0.0f && d->p.high_cut_hz < 0.45f * d->rate)
        st_biquad_lowpass(&d->high_cut, d->rate, d->p.high_cut_hz, 0.7071f);
    else
        st_biquad_identity(&d->high_cut);
}

static double delay_frames(const StDelay *d, float secs)
{
    double f = (double)secs * d->rate;
    double hi = (double)d->cap - 4.0;
    if (f < DELAY_MIN_FRAMES) f = DELAY_MIN_FRAMES;
    if (f > hi) f = hi;
    return f;
}

void st_delay_set_params(StDelay *d, const StDelayParams *p)
{
    d->p = *p;
    d->p.feedback = clampf(p->feedback, 0.0f, 0.95f);
    d->tgt_l = delay_frames(d, p->time_l);
    d->tgt_r = delay_frames(d, p->time_r);
    delay_filters(d);
}

int st_delay_init(StDelay *d, float rate, const StDelayParams *p, float max_seconds)
{
    if (!d) return -1;
    memset(d, 0, sizeof(*d));
    if (!p || !(rate >= 8000.0f) || !(max_seconds > 0.0f)) return -1;
    d->rate = rate;
    d->cap = (int)ceilf(max_seconds * rate) + 8;
    d->mem = calloc(2 * (size_t)d->cap, sizeof(float));
    if (!d->mem) {
        memset(d, 0, sizeof(*d));
        return -1;
    }
    d->buf_l = d->mem;
    d->buf_r = d->mem + d->cap;
    /* time constant 20 ms for retimed delays */
    d->glide = 1.0 - exp(-1.0 / (0.02 * rate));
    st_delay_set_params(d, p);
    d->cur_l = d->tgt_l;
    d->cur_r = d->tgt_r;
    st_biquad_reset(&d->low_cut);
    st_biquad_reset(&d->high_cut);
    return 0;
}

void st_delay_free(StDelay *d)
{
    if (!d) return;
    free(d->mem);
    memset(d, 0, sizeof(*d));
}

/* The input `frames` (>= 3, fractional) back from the next write at w. */
static float hermite_read(const float *buf, int cap, int w, double frames)
{
    double pos = (double)w - frames;
    int i;
    float f, x0, x1, x2, x3, c1, c2, c3;
    while (pos < 0.0) pos += cap;
    i = (int)pos;
    f = (float)(pos - i);
    x0 = buf[i == 0 ? cap - 1 : i - 1];
    x1 = buf[i];
    x2 = buf[i + 1 >= cap ? i + 1 - cap : i + 1];
    x3 = buf[i + 2 >= cap ? i + 2 - cap : i + 2];
    c1 = 0.5f * (x2 - x0);
    c2 = x0 - 2.5f * x1 + 2.0f * x2 - 0.5f * x3;
    c3 = 0.5f * (x3 - x0) + 1.5f * (x1 - x2);
    return ((c3 * f + c2) * f + c1) * f + x1;
}

void st_delay_process(StDelay *d, const float *in_l, const float *in_r,
                      float *out_l, float *out_r, int n)
{
    const float fb = d->p.feedback;
    for (int t = 0; t < n; t++) {
        float yl, yr, wl, wr;
        d->cur_l += (d->tgt_l - d->cur_l) * d->glide;
        d->cur_r += (d->tgt_r - d->cur_r) * d->glide;
        yl = hermite_read(d->buf_l, d->cap, d->w, d->cur_l);
        yr = hermite_read(d->buf_r, d->cap, d->w, d->cur_r);
        if (d->p.ping_pong) {
            wl = 0.5f * (in_l[t] + in_r[t]) + fb * yr;
            wr = fb * yl;
        } else {
            wl = in_l[t] + fb * yl;
            wr = in_r[t] + fb * yr;
        }
        wl = bq_tick(&d->high_cut, 0, bq_tick(&d->low_cut, 0, wl));
        wr = bq_tick(&d->high_cut, 1, bq_tick(&d->low_cut, 1, wr));
        d->buf_l[d->w] = flush_tiny(wl);
        d->buf_r[d->w] = flush_tiny(wr);
        if (++d->w >= d->cap) d->w = 0;
        out_l[t] += yl;
        out_r[t] += yr;
    }
}

/* ================================================================== */
/* Compressor.                                                        */
/*
 * D. Giannoulis, M. Massberg, J. D. Reiss, "Digital Dynamic Range
 * Compressor Design -- A Tutorial and Analysis", JAES 60(6), 2012:
 *   x_dB  = 20 log10 max(|key_l|, |key_r|)       (stereo-linked peak)
 *   y_dB  = gain computer with a soft knee of knee_db (their eq. 4)
 *   gr    = x_dB - y_dB                          (>= 0)
 *   gr_s  = their smooth decoupled peak detector (eq. 17), run on gr in
 *           the log domain (sec. 3.4):
 *             hold = max(gr, rel * hold + (1 - rel) * gr)
 *             gr_s = att * gr_s + (1 - att) * hold
 *           The first stage catches peaks at once and lets go at the
 *           release rate; the second smooths the result at the attack
 *           rate. On a steady tone the hold sits at the peak's reduction,
 *           so the static curve holds for peak level, ripple stays small,
 *           and the attack is not slowed by the waveform's zero crossings.
 *   gain  = 10^((makeup_db - gr_s) / 20)
 * Attack and release are time constants: a step in gain reduction is
 * 63% covered after about `attack` (rising) or `release` (falling;
 * slightly more, as the attack stage follows it) seconds.
 */

static double time_coef(float secs, float rate)
{
    if (secs <= 0.0f) return 0.0;
    return exp(-1.0 / ((double)secs * rate));
}

void st_comp_init(StComp *c, float rate, const StCompParams *p)
{
    memset(c, 0, sizeof(*c));
    c->rate = rate;
    c->p = *p;
    if (c->p.ratio < 1.0f) c->p.ratio = 1.0f;
    if (c->p.knee_db < 0.0f) c->p.knee_db = 0.0f;
    c->att_coef = time_coef(p->attack, rate);
    c->rel_coef = time_coef(p->release, rate);
}

static double comp_curve(const StCompParams *p, double x)
{
    double T = p->threshold_db, R = p->ratio, W = p->knee_db;
    double over = x - T;
    if (2.0 * over < -W) return x;
    if (W > 0.0 && 2.0 * fabs(over) <= W) {
        double k = over + W / 2.0;
        return x + (1.0 / R - 1.0) * k * k / (2.0 * W);
    }
    return T + over / R;
}

void st_comp_process(StComp *c, float *l, float *r, const float *key_l,
                     const float *key_r, int n)
{
    const float *kl = key_l ? key_l : l;
    const float *kr = key_l ? key_r : r;
    for (int t = 0; t < n; t++) {
        double lev = fabs((double)kl[t]), gr, gain;
        if (kr && fabs((double)kr[t]) > lev) lev = fabs((double)kr[t]);
        if (lev < 1e-9) lev = 1e-9; /* -180 dB */
        lev = 20.0 * log10(lev);
        gr = lev - comp_curve(&c->p, lev);
        c->hold_db = c->rel_coef * c->hold_db + (1.0 - c->rel_coef) * gr;
        if (gr > c->hold_db) c->hold_db = gr;
        c->gr_db = c->att_coef * c->gr_db + (1.0 - c->att_coef) * c->hold_db;
        gain = pow(10.0, (c->p.makeup_db - c->gr_db) / 20.0);
        l[t] = (float)(l[t] * gain);
        if (r) r[t] = (float)(r[t] * gain);
    }
}

float st_comp_gain_reduction_db(const StComp *c)
{
    return (float)c->gr_db;
}

/* ================================================================== */
/* Limiter.                                                           */
/*
 * After Geraint Luff, "Designing a straightforward limiter" (Signalsmith
 * Audio, 2022). With lookahead L frames, the output at frame n is the
 * input from frame n - L times a gain g[n], and g[n] must not exceed the
 * gain the peak at n - L requires, r[n-L] = min(1, ceiling / |x[n-L]|).
 *
 *   1. r[n]   = min(1, ceiling' / max(|l[n]|, |r[n]|))   (ceiling' is a
 *               hair under the ceiling, for float rounding)
 *   2. h[n]   = min(r[n-L .. n])           sliding minimum (a monotonic
 *               deque: O(1) amortised)
 *   3. e[n]   = h[n] if it is lower than e[n-1], else e[n-1] rising
 *               toward h[n] exponentially (the release); so e <= h
 *   4. g[n]   = mean(e[n-L .. n])          box average: a linear attack
 *               ramp across the lookahead
 *
 * Every e[k] in the box window k = n-L .. n is <= h[k], and every such
 * h[k] includes r[n-L] in its own window, so g[n] <= r[n-L]: the gain is
 * down by the time the peak leaves the delay line, by construction. The
 * final clamp exists only as a guard and counts its uses (tests expect
 * zero).
 */

int st_limiter_init(StLimiter *lm, float rate, float ceiling_db, float lookahead,
                    float release)
{
    size_t win, bytes;
    char *m;
    if (!lm) return -1;
    memset(lm, 0, sizeof(*lm));
    if (!(rate >= 8000.0f) || lookahead < 0.0f || lookahead > 1.0f) return -1;
    lm->rate = rate;
    lm->ceiling = st_db_to_gain(ceiling_db);
    lm->look = (int)lroundf(lookahead * rate);
    lm->rel_coef = release > 0.0f ? 1.0 - exp(-1.0 / ((double)release * rate)) : 1.0;
    win = (size_t)lm->look + 1;
    /* doubles first, then the 8-byte indices, then floats: all aligned */
    bytes = win * sizeof(double) + win * sizeof(long long) +
            win * sizeof(float) + 2 * win * sizeof(float);
    m = calloc(1, bytes);
    if (!m) {
        memset(lm, 0, sizeof(*lm));
        return -1;
    }
    lm->mem = m;
    lm->box = (double *)(void *)m;
    m += win * sizeof(double);
    lm->dq_idx = (long long *)(void *)m;
    m += win * sizeof(long long);
    lm->dq_val = (float *)(void *)m;
    m += win * sizeof(float);
    lm->dl_l = (float *)(void *)m;
    m += win * sizeof(float);
    lm->dl_r = (float *)(void *)m;
    lm->env = 1.0;
    for (size_t i = 0; i < win; i++) lm->box[i] = 1.0;
    lm->box_sum = (double)win;
    return 0;
}

void st_limiter_free(StLimiter *lm)
{
    if (!lm) return;
    free(lm->mem);
    memset(lm, 0, sizeof(*lm));
}

int st_limiter_latency(const StLimiter *lm)
{
    return lm->look;
}

void st_limiter_process(StLimiter *lm, float *l, float *r, int n)
{
    const int win = lm->look + 1;
    const float ceil_safe = lm->ceiling * (1.0f - 1e-6f);
    for (int i = 0; i < n; i++) {
        float xl = l[i], xr = r[i], pk, req, dlx, drx, yl, yr;
        double g;
        int back;

        /* 1. required gain for this frame */
        pk = fabsf(xl) > fabsf(xr) ? fabsf(xl) : fabsf(xr);
        req = pk > ceil_safe ? ceil_safe / pk : 1.0f;

        /* 2. sliding minimum over the last look + 1 frames */
        while (lm->dq_count > 0) {
            back = (lm->dq_head + lm->dq_count - 1) % win;
            if (lm->dq_val[back] >= req) lm->dq_count--;
            else break;
        }
        back = (lm->dq_head + lm->dq_count) % win;
        lm->dq_val[back] = req;
        lm->dq_idx[back] = lm->t;
        lm->dq_count++;
        while (lm->dq_idx[lm->dq_head] < lm->t - lm->look) {
            lm->dq_head = (lm->dq_head + 1) % win;
            lm->dq_count--;
        }

        /* 3. instant attack, exponential release */
        {
            double h = lm->dq_val[lm->dq_head];
            if (h < lm->env) lm->env = h;
            else lm->env += (h - lm->env) * lm->rel_coef;
        }

        /* 4. box average over look + 1 frames; the running sum is
         * recomputed exactly once per lap so rounding cannot drift */
        lm->box_sum += lm->env - lm->box[lm->box_pos];
        lm->box[lm->box_pos] = lm->env;
        if (++lm->box_pos >= win) {
            lm->box_pos = 0;
            lm->box_sum = 0.0;
            for (int k = 0; k < win; k++) lm->box_sum += lm->box[k];
        }
        g = lm->box_sum / win;
        if (g > 1.0) g = 1.0;

        /* the audio, delayed by `look` frames */
        if (lm->look > 0) {
            dlx = lm->dl_l[lm->dl_pos];
            drx = lm->dl_r[lm->dl_pos];
            lm->dl_l[lm->dl_pos] = xl;
            lm->dl_r[lm->dl_pos] = xr;
            if (++lm->dl_pos >= lm->look) lm->dl_pos = 0;
        } else {
            dlx = xl;
            drx = xr;
        }
        yl = (float)((double)dlx * g);
        yr = (float)((double)drx * g);
        if (fabsf(yl) > lm->ceiling) {
            yl = yl > 0.0f ? lm->ceiling : -lm->ceiling;
            lm->clamps++;
        }
        if (fabsf(yr) > lm->ceiling) {
            yr = yr > 0.0f ? lm->ceiling : -lm->ceiling;
            lm->clamps++;
        }
        l[i] = yl;
        r[i] = yr;
        lm->t++;
    }
}
