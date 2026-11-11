/*
 * studio_common.c: helpers every studio module shares (BRYX addition;
 * see studio.h).
 */
#include <math.h>

#include "studio.h"

uint32_t st_rand(uint32_t *state)
{
    uint32_t x = *state ? *state : 0x9E3779B9u;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

float st_randf(uint32_t *state)
{
    return (float)((double)st_rand(state) / 2147483648.0 - 1.0);
}

float st_db_to_gain(float db)
{
    return powf(10.0f, db / 20.0f);
}

float st_midi_to_hz(float note)
{
    return 440.0f * powf(2.0f, (note - 69.0f) / 12.0f);
}

/* An exponential segment covering 99% of the way in `secs`: per-sample
 * coefficient so that level += (target - level) * k. */
static float seg_coeff(float secs, float rate)
{
    if (secs <= 0.0f)
        return 1.0f;
    return 1.0f - expf(-4.605f / (secs * rate)); /* ln(100) = 4.605 */
}

void st_env_on(StEnv *e)
{
    e->stage = 1;
}

void st_env_off(StEnv *e)
{
    if (e->stage != 0)
        e->stage = 4;
}

float st_env_next(StEnv *e, const StAdsr *a, float rate)
{
    switch (e->stage) {
    case 1: /* attack: aim at 1.2 so the curve reaches 1.0 in `attack` */
        e->level += (1.2f - e->level) * seg_coeff(a->attack * 0.55f, rate);
        if (a->attack <= 0.0f || e->level >= 1.0f) {
            e->level = 1.0f;
            e->stage = 2;
        }
        break;
    case 2: /* decay toward sustain */
        e->level += (a->sustain - e->level) * seg_coeff(a->decay, rate);
        if (fabsf(e->level - a->sustain) < 1e-4f) {
            e->level = a->sustain;
            e->stage = 3;
        }
        break;
    case 3:
        e->level = a->sustain;
        break;
    case 4: /* release toward silence */
        e->level += (0.0f - e->level) * seg_coeff(a->release, rate);
        if (e->level < 1e-5f) {
            e->level = 0.0f;
            e->stage = 0;
        }
        break;
    default:
        e->level = 0.0f;
        break;
    }
    return e->level;
}

void st_synth_patch_default(StSynthPatch *p)
{
    StSynthPatch d = {0};
    d.wave = ST_WAVE_SAW;
    d.pulse_width = 0.5f;
    d.stack = 1;
    d.amp = (StAdsr){0.005f, 0.2f, 0.8f, 0.25f};
    d.filter = ST_FILTER_NONE;
    d.cutoff_hz = 8000.0f;
    d.resonance = 0.1f;
    d.key_track = 0.5f;
    d.filter_env = (StAdsr){0.005f, 0.3f, 0.0f, 0.3f};
    d.pitch_env_time = 0.04f;
    d.lfo[0].rate_hz = 5.5f;
    d.lfo[1].rate_hz = 0.2f;
    *p = d;
}

void st_sampler_patch_default(StSamplerPatch *p)
{
    StSamplerPatch d = {0};
    d.amp = (StAdsr){0.002f, 0.0f, 1.0f, 0.35f};
    d.velocity_curve = 1.7f;
    d.stack = 1;
    *p = d;
}
