#include "audio_mix.h"
#include "audio_dsp.h"
#include "audio_fx.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

#define MIX_VOICES_PER_TRACK 3
/* BRYX: a song that writes chords gets more voices per track, so a held
 * triad's release tails ring under the next chord instead of being
 * stolen. Songs without chords keep three, and render exactly as before. */
#define MIX_MAX_VOICES_PER_TRACK 6
#define MIX_MAX_STACKED_OSC  2

typedef struct {
    int active;
    int instrument_index;
    int note;
    int fx_bus;
    int fx_trigger;
    int accent;
    int osc_count;
    int detune_cents;
    int base_amplitude;
    int pulse_width;
    int pwm_depth;
    int pwm_rate;
    int vibrato_depth;
    int vibrato_rate;
    int vibrato_ramp;     /* BRYX: samples over which vibrato fades in; 0 = at once */
    int age;              /* BRYX: samples since the note-on */
    int pitch_cents;      /* BRYX: the pitch envelope's start offset */
    int pitch_len;        /* BRYX: and its length in samples; 0 = none */
    int sample_rate;
    DspWaveform waveform;
    DspOscillator oscs[MIX_MAX_STACKED_OSC];
    DspEnvelopeRuntime env;
    double target_freq;
    double current_freq;
    double glide_step;
    int glide_remaining;
    uint32_t vibrato_phase;
    uint32_t pwm_phase;
} MixVoiceState;

typedef struct {
    MixVoiceState voices[MIX_MAX_VOICES_PER_TRACK];
    int voice_limit;      /* BRYX: MIX_VOICES_PER_TRACK, or more for chords */
    double last_freq;
    int duty_override;    /* BRYX: a %%ramp's pulse width for new notes; 0 = the instrument's */
} MixTrackState;

static DspWaveform sanitize_waveform(int waveform, int noise_mode)
{
    if (noise_mode) return DSP_WAVE_NOISE;
    if (waveform < DSP_WAVE_SQUARE || waveform > DSP_WAVE_NOISE)
        return DSP_WAVE_SQUARE;
    return (DspWaveform)waveform;
}

static double midi_note_to_freq(int note)
{
    static const double midi_freq_table[128] = {
        8.175799, 8.661957, 9.177024, 9.722718, 10.300861, 10.913382, 11.562326, 12.249857,
        12.978272, 13.750000, 14.567618, 15.433853, 16.351598, 17.323914, 18.354048, 19.445436,
        20.601722, 21.826764, 23.124651, 24.499715, 25.956544, 27.500000, 29.135235, 30.867706,
        32.703196, 34.647829, 36.708096, 38.890873, 41.203445, 43.653529, 46.249303, 48.999429,
        51.913087, 55.000000, 58.270470, 61.735413, 65.406391, 69.295658, 73.416192, 77.781746,
        82.406889, 87.307058, 92.498606, 97.998859, 103.826174, 110.000000, 116.540940, 123.470825,
        130.812783, 138.591315, 146.832384, 155.563492, 164.813778, 174.614116, 184.997211, 195.997718,
        207.652349, 220.000000, 233.081881, 246.941651, 261.625565, 277.182631, 293.664768, 311.126984,
        329.627557, 349.228231, 369.994423, 391.995436, 415.304698, 440.000000, 466.163762, 493.883301,
        523.251131, 554.365262, 587.329536, 622.253967, 659.255114, 698.456463, 739.988845, 783.990872,
        830.609395, 880.000000, 932.327523, 987.766603, 1046.502261, 1108.730524, 1174.659072, 1244.507935,
        1318.510228, 1396.912926, 1479.977691, 1567.981744, 1661.218790, 1760.000000, 1864.655046, 1975.533205,
        2093.004522, 2217.461048, 2349.318143, 2489.015870, 2637.020455, 2793.825851, 2959.955382, 3135.963488,
        3322.437581, 3520.000000, 3729.310092, 3951.066410, 4186.009045, 4434.922096, 4698.636287, 4978.031740,
        5274.040911, 5587.651703, 5919.910763, 6271.926976, 6644.875161, 7040.000000, 7458.620184, 7902.132820,
        8372.018090, 8869.844191, 9397.272573, 9956.063479, 10548.081821, 11175.303406, 11839.821527, 12543.853951
    };
    if (note < 0 || note > 127) return 0.0;
    return midi_freq_table[note];
}

static SeqFxBus normalize_fx_bus(const SeqFxBus *bus)
{
    SeqFxBus out;
    if (!bus) {
        memset(&out, 0, sizeof(out));
        return out;
    }
    out = *bus;
    if (out.enabled != 0 && out.enabled != 1) {
        int legacy_drive_from_enabled = out.enabled;
        int legacy_mix_from_delay_steps = out.delay_steps;
        memset(&out, 0, sizeof(out));
        out.enabled = 1;
        out.drive_amount = legacy_drive_from_enabled;
        out.mix_percent = legacy_mix_from_delay_steps;
    }
    if (out.mix_percent <= 0 && out.enabled &&
        (out.delay_mix > 0 || out.drive_amount > 0 || out.lowpass_amount > 0 || out.sidechain_amount > 0))
        out.mix_percent = 100;
    out.mix_percent = dsp_clampi(out.mix_percent, 0, 100);
    out.delay_feedback = dsp_clampi(out.delay_feedback, 0, 95);
    out.delay_mix = dsp_clampi(out.delay_mix, 0, 100);
    out.drive_amount = dsp_clampi(out.drive_amount, 0, 100);
    out.lowpass_amount = dsp_clampi(out.lowpass_amount, 0, 100);
    out.sidechain_amount = dsp_clampi(out.sidechain_amount, 0, 100);
    if (out.sidechain_release_ms <= 0) out.sidechain_release_ms = 180;
    return out;
}

static int track_amplitude(const SeqSong *song, const SeqTrack *track,
                           const SeqStep *step, int pattern_step)
{
    int amplitude;
    const SeqInstrument *instrument = &song->instruments[track->instrument];

    amplitude = (instrument->amplitude * step->velocity) / 127;
    amplitude = (amplitude * (100 + track->automation[pattern_step])) / 100;
    if (step->accent)
        amplitude = (amplitude * (100 + instrument->accent_gain)) / 100;
    return dsp_clampi(amplitude, 0, 127);
}

/* BRYX: `taken` marks slots already struck this step (a chord's notes),
 * which a later note of the same chord must not steal: a voice that has
 * just started its attack is the quietest of all. */
static MixVoiceState *pick_voice_slot(MixTrackState *track, unsigned *taken)
{
    int pick = -1;
    for (int i = 0; i < track->voice_limit; i++) {
        if (*taken & (1u << i))
            continue;
        if (!track->voices[i].active) {
            pick = i;
            break;
        }
        if (pick < 0 || track->voices[i].env.level_q15 < track->voices[pick].env.level_q15)
            pick = i;
    }
    if (pick < 0) pick = 0;
    *taken |= 1u << pick;
    return &track->voices[pick];
}

static void voice_set_frequency(MixVoiceState *voice, int sample_rate)
{
    int vib = (voice->vibrato_depth > 0 && voice->vibrato_rate > 0)
        ? (dsp_tri_lfo_q15(&voice->vibrato_phase, voice->vibrato_rate, sample_rate) * voice->vibrato_depth) / 32767
        : 0;
    if (voice->vibrato_ramp > 0 && voice->age < voice->vibrato_ramp)
        vib = (int)(((int64_t)vib * voice->age) / voice->vibrato_ramp);
    if (voice->pitch_len > 0 && voice->age < voice->pitch_len) {
        /* BRYX: the pitch envelope falls on a square-law curve: fast, then settling. */
        double left = 1.0 - (double)voice->age / (double)voice->pitch_len;
        vib += (int)(voice->pitch_cents * left * left);
    }
    double base = dsp_freq_with_cents(voice->current_freq, vib);

    if (voice->osc_count == 1) {
        dsp_osc_set_frequency(&voice->oscs[0], base, sample_rate);
    } else {
        int detune = dsp_clampi(voice->detune_cents, 0, 200);
        dsp_osc_set_frequency(&voice->oscs[0], dsp_freq_with_cents(base, -detune), sample_rate);
        dsp_osc_set_frequency(&voice->oscs[1], dsp_freq_with_cents(base, detune), sample_rate);
    }
}

static void voice_init(MixVoiceState *voice, const SeqInstrument *instrument,
                       int sample_rate, int amplitude, double target_freq,
                       double from_freq, int gate_samples, int instrument_index,
                       int note, int fx_trigger, int accent)
{
    int pulse = dsp_clampi(instrument->pulse_width, 1, 99);
    int glide_samples = dsp_samples_from_ms(sample_rate, instrument->glide_ms);

    memset(voice, 0, sizeof(*voice));
    voice->active = 1;
    voice->instrument_index = instrument_index;
    voice->note = note;
    voice->fx_bus = instrument->fx_send;
    voice->fx_trigger = fx_trigger;
    voice->accent = accent;
    voice->detune_cents = instrument->detune_cents;
    voice->base_amplitude = amplitude;
    voice->pulse_width = pulse;
    voice->pwm_depth = dsp_clampi(instrument->pwm_depth, 0, 40);
    voice->pwm_rate = instrument->pwm_rate;
    voice->vibrato_depth = dsp_clampi(instrument->vibrato_depth_cents, 0, 120);
    voice->vibrato_rate = instrument->vibrato_rate;
    voice->vibrato_ramp = dsp_samples_from_ms(sample_rate, instrument->vibrato_delay_ms);
    voice->pitch_cents = instrument->pitch_env_cents;
    voice->pitch_len = instrument->pitch_env_cents ? dsp_samples_from_ms(sample_rate, instrument->pitch_env_ms) : 0;
    voice->sample_rate = sample_rate;
    voice->waveform = sanitize_waveform(instrument->waveform, instrument->noise_mode);
    voice->osc_count = (instrument->stacked_voices > 1) ? 2 : 1;
    if (voice->waveform == DSP_WAVE_NOISE) voice->osc_count = 1;
    if (voice->osc_count < 1) voice->osc_count = 1;
    if (voice->osc_count > MIX_MAX_STACKED_OSC) voice->osc_count = MIX_MAX_STACKED_OSC;

    for (int i = 0; i < voice->osc_count; i++) {
        /* BRYX: the oscillator runs at the engine's internal scale; halving
         * for a stacked pair happens there, so an odd amplitude keeps its
         * last half step. */
        int amp = (voice->osc_count == 1) ? (amplitude << DSP_FRAC_BITS) : ((amplitude << DSP_FRAC_BITS) / 2);
        dsp_osc_init(&voice->oscs[i], voice->waveform, amp);
        voice->oscs[i].noise_clock = instrument->noise_clock;
        voice->oscs[i].bandlimit = instrument->bandlimit;
        if (voice->waveform == DSP_WAVE_PULSE)
            dsp_osc_set_pulse_width_percent(&voice->oscs[i], pulse);
    }

    voice->target_freq = target_freq;
    voice->current_freq = (from_freq > 0.0 && glide_samples > 0) ? from_freq : target_freq;
    voice->glide_remaining = (from_freq > 0.0 && glide_samples > 0) ? glide_samples : 0;
    voice->glide_step = (voice->glide_remaining > 0)
        ? (target_freq - voice->current_freq) / (double)voice->glide_remaining
        : 0.0;
    voice_set_frequency(voice, sample_rate);
    dsp_envelope_init(&voice->env, &instrument->envelope, sample_rate, gate_samples);
}

static int voice_next(MixVoiceState *voice)
{
    int env_q15;
    int mixed = 0;

    if (!voice->active) return 0;
    voice->age++;
    if (voice->glide_remaining > 0) {
        voice->current_freq += voice->glide_step;
        voice->glide_remaining--;
        if (voice->glide_remaining <= 0)
            voice->current_freq = voice->target_freq;
    }
    voice_set_frequency(voice, voice->sample_rate);

    if (voice->waveform == DSP_WAVE_PULSE && voice->pwm_depth > 0 && voice->pwm_rate > 0) {
        int mod = (dsp_tri_lfo_q15(&voice->pwm_phase, voice->pwm_rate, voice->sample_rate) * voice->pwm_depth) / 32767;
        int duty = dsp_clampi(voice->pulse_width + mod, 1, 99);
        for (int i = 0; i < voice->osc_count; i++)
            dsp_osc_set_pulse_width_percent(&voice->oscs[i], duty);
    }

    env_q15 = dsp_envelope_next_q15(&voice->env);
    if (voice->env.phase == DSP_ENV_IDLE) {
        voice->active = 0;
        return 0;
    }

    for (int i = 0; i < voice->osc_count; i++)
        mixed += dsp_osc_next(&voice->oscs[i]);
    mixed = (mixed + (voice->osc_count / 2)) / voice->osc_count;
    return (mixed * env_q15) / 32767;
}

/* BRYX: the render core. Writes signed 16-bit samples: the engine's
 * internal scale (8-bit steps shifted by DSP_FRAC_BITS) is exactly 16-bit
 * full scale, so nothing is lost until the caller chooses a format. */
int16_t *audio_mix_render_timeline_s16(const SeqSong *song, const SeqTimeline *timeline,
                                       int sample_rate, int *out_len)
{
    int16_t *buf;
    MixTrackState tracks[SEQ_MAX_TRACKS];
    AudioFxBusState fx_states[SEQ_MAX_FX_BUSES];
    int fx_bus_levels[SEQ_MAX_FX_BUSES];
    int fx_bus_trigger[SEQ_MAX_FX_BUSES];
    int key_struck[SEQ_MAX_FX_BUSES];
    int master_permille;
    int base = 0;

    if (!song || !timeline || !out_len || timeline->total_steps <= 0 || timeline->total_samples <= 0)
        return NULL;

    buf = malloc((size_t)timeline->total_samples * sizeof(int16_t));
    if (!buf) return NULL;
    memset(tracks, 0, sizeof(tracks));
    for (int t = 0; t < SEQ_MAX_TRACKS; t++)
        tracks[t].voice_limit = song->chord_voices ? MIX_MAX_VOICES_PER_TRACK : MIX_VOICES_PER_TRACK;
    memset(fx_states, 0, sizeof(fx_states));
    master_permille = song->master_permille > 0 ? song->master_permille : 1000;

    for (int bus_index = 0; bus_index < song->fx_bus_count && bus_index < SEQ_MAX_FX_BUSES; bus_index++) {
        SeqFxBus cfg = normalize_fx_bus(&song->fx_buses[bus_index]);
        if (audio_fx_bus_init(&fx_states[bus_index], &cfg, sample_rate, song->tempo_bpm, song->steps_per_beat) != 0) {
            for (int i = 0; i < song->fx_bus_count && i < SEQ_MAX_FX_BUSES; i++)
                audio_fx_bus_free(&fx_states[i]);
            free(buf);
            return NULL;
        }
    }

    /* BRYX: under %%loop the song plays twice and only the second pass is
     * kept, so its start carries the end's echoes, reverb and release
     * tails: a loop with no seam. Ramps and voices run on across passes. */
    for (int pass = song->loop ? 0 : 1; pass < 2; pass++) {
        int keep = pass == 1;
        base = 0;
        for (int absolute_step = 0; absolute_step < timeline->total_steps; absolute_step++) {
            SeqNoteEvent events[SEQ_MAX_STEP_EVENTS];
            int event_count = seq_collect_step_events(song, timeline, absolute_step, events);
            const SeqTimelineStep *timeline_step = &timeline->steps[absolute_step];
            const SeqPattern *pattern = &song->patterns[timeline_step->pattern_index];
            int samples_this_step = timeline_step->samples;

            /* BRYX: ramps move their parameters at each step they cover. */
            for (int r = 0; r < song->ramp_count; r++) {
                const SeqRamp *ramp = &song->ramps[r];
                int span, value;
                if (absolute_step < ramp->start_step || absolute_step > ramp->end_step)
                    continue;
                span = ramp->end_step - ramp->start_step;
                value = span > 0
                    ? ramp->from + (int)(((int64_t)(ramp->to - ramp->from) * (absolute_step - ramp->start_step)) / span)
                    : ramp->to;
                if (ramp->param == SEQ_RAMP_DUTY) {
                    if (ramp->track >= 0 && ramp->track < SEQ_MAX_TRACKS)
                        tracks[ramp->track].duty_override = dsp_clampi(value, 1, 99);
                } else if (ramp->bus >= 0 && ramp->bus < song->fx_bus_count) {
                    audio_fx_bus_set_param(&fx_states[ramp->bus], ramp->param, value);
                }
            }

            memset(key_struck, 0, sizeof(key_struck));
            for (int e = 0; e < event_count; e++) {
                const SeqNoteEvent *event = &events[e];
                const SeqTrack *track = &pattern->tracks[event->track_index];
                const SeqStep *step = &track->steps[event->step_index];
                SeqInstrument played = song->instruments[event->instrument_index];
                const SeqInstrument *instrument = &played;
                int amp = track_amplitude(song, track, step, event->step_index);
                unsigned taken = 0;
                int chord[2];
                /* BRYX: a held note's gate spans every step tied to it. */
                int held = seq_held_samples(song, timeline, absolute_step, event->track_index);
                int gate_samples = (int)(((int64_t)held * event->gate_percent) / 100);
                double target_freq = midi_note_to_freq(event->note);
                MixTrackState *track_state = &tracks[event->track_index];
                MixVoiceState *voice = pick_voice_slot(track_state, &taken);

                if (track_state->duty_override > 0)
                    played.pulse_width = track_state->duty_override;
                voice_init(voice, instrument, sample_rate, amp, target_freq,
                           track_state->last_freq, gate_samples, event->instrument_index,
                           event->note, event->fx_trigger, event->accent);
                /* BRYX: a chord's other notes take the track's other voices,
                 * struck together and never gliding. */
                chord[0] = event->note2;
                chord[1] = event->note3;
                for (int c = 0; c < 2; c++) {
                    if (chord[c] <= 0) continue;
                    voice_init(pick_voice_slot(track_state, &taken), instrument, sample_rate, amp,
                               midi_note_to_freq(chord[c]), 0.0, gate_samples, event->instrument_index,
                               chord[c], event->fx_trigger, event->accent);
                }
                track_state->last_freq = target_freq;
                for (int b = 0; b < song->fx_bus_count; b++)
                    if (song->fx_buses[b].sidechain_key == event->track_index + 1)
                        key_struck[b] = 1;
            }

            for (int i = 0; i < samples_this_step; i++) {
                int pre_fx_sum = 0;
                int fx_return_sum = 0;
                int master_sidechain_env_q15 = 32767;
                int64_t out;
                memset(fx_bus_levels, 0, sizeof(fx_bus_levels));
                memset(fx_bus_trigger, 0, sizeof(fx_bus_trigger));

                for (int track_index = 0; track_index < timeline->max_track_count; track_index++) {
                    for (int v = 0; v < tracks[track_index].voice_limit; v++) {
                        MixVoiceState *voice = &tracks[track_index].voices[v];
                        int sample;
                        int on_bus;
                        if (!voice->active) continue;
                        sample = voice_next(voice);
                        on_bus = voice->fx_bus >= 0 && voice->fx_bus < song->fx_bus_count;
                        /* BRYX: an insert bus takes its voices' dry signal. */
                        if (!on_bus || !song->fx_buses[voice->fx_bus].insert)
                            pre_fx_sum += sample;
                        if (on_bus) {
                            int trig = voice->fx_trigger > 0 ? voice->fx_trigger : 0;
                            if (voice->accent > 0 && trig < 1) trig = 1;
                            fx_bus_levels[voice->fx_bus] += sample;
                            if (trig > fx_bus_trigger[voice->fx_bus])
                                fx_bus_trigger[voice->fx_bus] = trig;
                        }
                    }
                }

                for (int bus_index = 0; bus_index < song->fx_bus_count; bus_index++) {
                    int sc_env = 32767;
                    int keyed = song->fx_buses[bus_index].sidechain_key > 0;
                    /* BRYX: a keyed sidechain fires once, on its key's note-on. */
                    int trigger = keyed ? (i == 0 && key_struck[bus_index]) : fx_bus_trigger[bus_index];
                    fx_return_sum += audio_fx_bus_process(&fx_states[bus_index], fx_bus_levels[bus_index],
                                                          trigger, &sc_env);
                    if (!keyed && sc_env < master_sidechain_env_q15)
                        master_sidechain_env_q15 = sc_env;
                }

                out = ((int64_t)(pre_fx_sum + fx_return_sum) * master_sidechain_env_q15) / 32767;
                out = (out * master_permille) / 1000;
                if (out > 32767) out = 32767;
                if (out < -32768) out = -32768;
                if (keep)
                    buf[base + i] = (int16_t)out;
            }
            base += samples_this_step;
        }
    }

    for (int i = 0; i < song->fx_bus_count && i < SEQ_MAX_FX_BUSES; i++)
        audio_fx_bus_free(&fx_states[i]);
    *out_len = timeline->total_samples;
    return buf;
}

/* The original 8-bit output, now quantized once from the 16-bit mix with
 * triangular dither (a fixed-seed generator, so renders stay
 * deterministic) instead of at every voice and every multiply. */
unsigned char *audio_mix_render_timeline(const SeqSong *song, const SeqTimeline *timeline,
                                         int sample_rate, int *out_len)
{
    int len = 0;
    int16_t *wide = audio_mix_render_timeline_s16(song, timeline, sample_rate, &len);
    unsigned char *buf;
    uint32_t rng = 0x9e3779b9u;

    if (!wide) return NULL;
    buf = malloc((size_t)len);
    if (!buf) {
        free(wide);
        return NULL;
    }
    for (int i = 0; i < len; i++) {
        int d;
        rng = rng * 1664525u + 1013904223u;
        d = (int)((rng >> 24) & 0xff);
        rng = rng * 1664525u + 1013904223u;
        d -= (int)((rng >> 24) & 0xff);
        buf[i] = (unsigned char)dsp_clamp_u8(128 + (int)floor(((double)wide[i] + d) / 256.0 + 0.5));
    }
    free(wide);
    *out_len = len;
    return buf;
}

int16_t *audio_mix_render_song_s16(const SeqSong *song, int sample_rate, int *out_len)
{
    SeqTimeline *timeline = calloc(1, sizeof(*timeline)); /* BRYX: heap, see audio_engine.c */
    int16_t *pcm = NULL;

    if (timeline && seq_compile_timeline(song, sample_rate, timeline) == 0)
        pcm = audio_mix_render_timeline_s16(song, timeline, sample_rate, out_len);
    free(timeline);
    return pcm;
}

unsigned char *audio_mix_render_song(const SeqSong *song, int sample_rate, int *out_len)
{
    SeqTimeline *timeline = calloc(1, sizeof(*timeline));
    unsigned char *pcm = NULL;

    if (timeline && seq_compile_timeline(song, sample_rate, timeline) == 0)
        pcm = audio_mix_render_timeline(song, timeline, sample_rate, out_len);
    free(timeline);
    return pcm;
}
