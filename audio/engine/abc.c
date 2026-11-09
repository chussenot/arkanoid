#include "electric_pulse.h"
#include <limits.h>
#include "audio_dsp.h"
#include "audio_mix.h"

/*
 * Minimal ABC notation parser for the Electric Pulse chiptune engine.
 *
 * Supports:
 *   - Header fields: X, T, M, L, Q, K, V (with amp/wave/duty/ADSR/vibrato/glide directives)
 *   - Notes: C-B (octave 4), c-b (octave 5), with , (down) and ' (up) modifiers
 *   - Accidentals: ^ (sharp), _ (flat), = (natural)
 *   - Rests: z
 *   - Note lengths: multipliers (e.g. A2 = twice default length)
 *   - Repeat markers: |: and :|
 *   - Multi-voice via V: lines
 */

/* ─── Note frequency table ───────────────────────────────────── */

/* semitone offsets from C: C=0, D=2, E=4, F=5, G=7, A=9, B=11 */
static const int note_semitones[7] = { 0, 2, 4, 5, 7, 9, 11 };
#define ABC_DEFAULT_VIBRATO_RATE 5500
#define ABC_EFFECT_DELAY "delay"
#define ABC_EFFECT_DRIVE "drive"
#define ABC_EFFECT_LOWPASS "lowpass"
#define ABC_TO_SEQ_AMP_PCT 60
#define ABC_DEFAULT_SIDECHAIN_RELEASE_MS 180
#define ABC_DEFAULT_FX_BUS_COUNT 1

/*
 * Precomputed MIDI note frequencies (Hz).
 * Index = MIDI note number (0–127). A4 = index 69 = 440.000000 Hz.
 * Generated via: 440.0 * pow(2.0, (i - 69) / 12.0)
 * Eliminates per-note pow() calls during parsing.
 */
static const double midi_freq_table[128] = {
    8.175799, 8.661957, 9.177024, 9.722718,
    10.300861, 10.913382, 11.562326, 12.249857,
    12.978272, 13.750000, 14.567618, 15.433853,
    16.351598, 17.323914, 18.354048, 19.445436,
    20.601722, 21.826764, 23.124651, 24.499715,
    25.956544, 27.500000, 29.135235, 30.867706,
    32.703196, 34.647829, 36.708096, 38.890873,
    41.203445, 43.653529, 46.249303, 48.999429,
    51.913087, 55.000000, 58.270470, 61.735413,
    65.406391, 69.295658, 73.416192, 77.781746,
    82.406889, 87.307058, 92.498606, 97.998859,
    103.826174, 110.000000, 116.540940, 123.470825,
    130.812783, 138.591315, 146.832384, 155.563492,
    164.813778, 174.614116, 184.997211, 195.997718,
    207.652349, 220.000000, 233.081881, 246.941651,
    261.625565, 277.182631, 293.664768, 311.126984,
    329.627557, 349.228231, 369.994423, 391.995436,
    415.304698, 440.000000, 466.163762, 493.883301,
    523.251131, 554.365262, 587.329536, 622.253967,
    659.255114, 698.456463, 739.988845, 783.990872,
    830.609395, 880.000000, 932.327523, 987.766603,
    1046.502261, 1108.730524, 1174.659072, 1244.507935,
    1318.510228, 1396.912926, 1479.977691, 1567.981744,
    1661.218790, 1760.000000, 1864.655046, 1975.533205,
    2093.004522, 2217.461048, 2349.318143, 2489.015870,
    2637.020455, 2793.825851, 2959.955382, 3135.963488,
    3322.437581, 3520.000000, 3729.310092, 3951.066410,
    4186.009045, 4434.922096, 4698.636287, 4978.031740,
    5274.040911, 5587.651703, 5919.910763, 6271.926976,
    6644.875161, 7040.000000, 7458.620184, 7902.132820,
    8372.018090, 8869.844191, 9397.272573, 9956.063479,
    10548.081821, 11175.303406, 11839.821527, 12543.853951
};

static double semitone_to_freq(int midi_note)
{
    if (midi_note < 0 || midi_note > 127) return 0.0;
    return midi_freq_table[midi_note];
}

static int closest_midi_from_freq(double freq)
{
    int lo = 0;
    int hi = 127;
    int mid;
    double dlo;
    double dhi;
    if (freq <= midi_freq_table[0]) return 0;
    if (freq >= midi_freq_table[127]) return 127;
    while (hi - lo > 1) {
        mid = lo + (hi - lo) / 2;
        if (midi_freq_table[mid] <= freq)
            lo = mid;
        else
            hi = mid;
    }
    dlo = freq - midi_freq_table[lo];
    if (dlo < 0.0) dlo = -dlo;
    dhi = freq - midi_freq_table[hi];
    if (dhi < 0.0) dhi = -dhi;
    return (dlo <= dhi) ? lo : hi;
}

static void clear_seq_track(SeqTrack *track)
{
    if (!track) return;
    memset(track, 0, sizeof(*track));
    for (int s = 0; s < SEQ_MAX_STEPS; s++)
        track->steps[s].note = SEQ_NOTE_REST;
}

static int default_steps_per_beat(int tempo_bpm, int step_ms)
{
    int denom;
    int spb;

    if (tempo_bpm <= 0 || step_ms <= 0)
        return 4;
    denom = tempo_bpm * step_ms;
    if (denom <= 0)
        return 4;
    spb = (60000 + denom / 2) / denom;
    if (spb < 1) spb = 1;
    if (spb > 16) spb = 16;
    return spb;
}

static void fill_track_steps(SeqTrack *track, const AbcVoice *voice,
                             int source_step_base, int length, int steps_per_beat, int hold)
{
    int gate_percent;

    if (!track || !voice) return;
    gate_percent = dsp_clampi(voice->gate_percent > 0 ? voice->gate_percent : (voice->staccato ? 75 : 90), 1, 100);
    clear_seq_track(track);
    for (int s = 0; s < length && s < SEQ_MAX_STEPS; s++) {
        int source_step = source_step_base + s;
        SeqStep *step = &track->steps[s];

        step->gate = (uint8_t)gate_percent;
        if (source_step >= voice->note_count || voice->freqs[source_step] <= 0.0) {
            step->gate = 0;
            continue;
        }

        step->note = (int16_t)closest_midi_from_freq(voice->freqs[source_step]);
        /* BRYX: written dynamics and chord notes. */
        step->velocity = voice->vel[source_step] ? voice->vel[source_step] : 88;
        if (voice->freqs2[source_step] > 0.0)
            step->note2 = (int16_t)closest_midi_from_freq(voice->freqs2[source_step]);
        if (voice->freqs3[source_step] > 0.0)
            step->note3 = (int16_t)closest_midi_from_freq(voice->freqs3[source_step]);
        if (voice->extras.accents_marked)
            step->accent = voice->acc[source_step];
        else
            step->accent = (uint8_t)((steps_per_beat > 0 && (s % steps_per_beat) == 0) ? 1 : 0);
        step->fx_trigger = step->accent;
        /* BRYX: under %%hold, a step inside a written note continues it. */
        if (hold && source_step > 0 && !voice->onset[source_step] &&
            voice->freqs[source_step - 1] == voice->freqs[source_step] &&
            voice->freqs2[source_step - 1] == voice->freqs2[source_step] &&
            voice->freqs3[source_step - 1] == voice->freqs3[source_step])
            step->tie = 1;
    }
}

static int append_pattern_instance(SeqSong *song, const AbcMusic *music, int track_count,
                                   int pattern_length, int source_step_base, int *arrangement_written)
{
    SeqPattern *pattern;

    if (!song || !music || !arrangement_written)
        return -1;
    if (song->pattern_count >= SEQ_MAX_PATTERNS || *arrangement_written >= SEQ_MAX_ARRANGEMENT)
        return -1;
    if (pattern_length < 1)
        pattern_length = 1;
    if (pattern_length > SEQ_MAX_STEPS)
        pattern_length = SEQ_MAX_STEPS;

    pattern = &song->patterns[song->pattern_count];
    memset(pattern, 0, sizeof(*pattern));
    pattern->length = pattern_length;
    pattern->track_count = track_count;
    for (int t = 0; t < track_count; t++) {
        fill_track_steps(&pattern->tracks[t], &music->voices[t], source_step_base, pattern_length,
                         song->steps_per_beat, music->hold);
        pattern->tracks[t].instrument = t;
    }

    song->arrangement[*arrangement_written] = (uint8_t)song->pattern_count;
    song->pattern_count++;
    (*arrangement_written)++;
    return 0;
}

static int note_char_to_index(char c)
{
    /* C=0, D=1, E=2, F=3, G=4, A=5, B=6 */
    switch (c) {
        case 'C': case 'c': return 0;
        case 'D': case 'd': return 1;
        case 'E': case 'e': return 2;
        case 'F': case 'f': return 3;
        case 'G': case 'g': return 4;
        case 'A': case 'a': return 5;
        case 'B': case 'b': return 6;
    }
    return -1;
}

static DspWaveform sanitize_waveform(int waveform)
{
    if (waveform < DSP_WAVE_SQUARE || waveform > DSP_WAVE_NOISE)
        return DSP_WAVE_SQUARE;
    return (DspWaveform)waveform;
}

static int parse_waveform_name(const char *s)
{
    if (!s) return DSP_WAVE_SQUARE;
    if (strncmp(s, "pulse", 5) == 0) return DSP_WAVE_PULSE;
    if (strncmp(s, "triangle", 8) == 0) return DSP_WAVE_TRIANGLE;
    if (strncmp(s, "noise", 5) == 0) return DSP_WAVE_NOISE;
    return DSP_WAVE_SQUARE;
}

static int validate_waveform_name(const char *name)
{
    if (!name) return 0;
    if (strncmp(name, "square", 6) == 0) return 1;
    if (strncmp(name, "pulse", 5) == 0) return 1;
    if (strncmp(name, "triangle", 8) == 0) return 1;
    if (strncmp(name, "noise", 5) == 0) return 1;
    if (strncmp(name, "sine", 4) == 0) return 1;
    return 0;
}

static int validate_amplitude(int amp)
{
    return (amp >= 0 && amp <= 127);
}

static int validate_duty_cycle(int duty)
{
    return (duty >= 1 && duty <= 99);
}

/* Reserved for future FX bus routing feature (%%instrument bass fx=0)
 * Currently unused but defined to maintain validation pattern consistency */
__attribute__((unused))
static int validate_fx_bus(int fx_bus)
{
    return (fx_bus >= 0 && fx_bus < 4);  /* SEQ_MAX_FX_BUSES */
}

static int parse_int_param(const char *text, const char *key, int fallback);

/* BRYX: the parameters in AbcExtras, from a %%instrument or V: line. */
static void parse_extras(const char *val, AbcExtras *x)
{
    static const char *keys[] = { "detune=", "stack=", "pwm=", "pwm_rate=",
                                  "vibrato_rate=", "vibrato_delay=", "accent=",
                                  "pitch=", "pitch_time=" };
    int *fields[] = { &x->detune_cents, &x->stack, &x->pwm_depth, &x->pwm_rate,
                      &x->vibrato_rate, &x->vibrato_delay_ms, &x->accent_gain,
                      &x->pitch, &x->pitch_time };
    const char *noise = strstr(val, "noise=");
    const char *accents = strstr(val, "accents=");
    for (int i = 0; i < 9; i++) {
        int value = parse_int_param(val, keys[i], INT_MIN);
        if (value != INT_MIN) {
            *fields[i] = value;
            x->set |= 1u << i;
        }
    }
    if (noise) {
        x->noise = strncmp(noise + 6, "metal", 5) == 0 ? 2 : strncmp(noise + 6, "long", 4) == 0 ? 1 : 0;
        x->set |= 1u << 9;
    }
    if (accents) {
        x->accents_marked = strncmp(accents + 8, "marked", 6) == 0;
        x->set |= 1u << 10;
    }
}

/* BRYX: a voice takes each extra it did not set from its instrument. */
static void merge_extras(AbcExtras *voice, const AbcExtras *inst)
{
    int *dst[] = { &voice->detune_cents, &voice->stack, &voice->pwm_depth, &voice->pwm_rate,
                   &voice->vibrato_rate, &voice->vibrato_delay_ms, &voice->accent_gain,
                   &voice->pitch, &voice->pitch_time, &voice->noise, &voice->accents_marked };
    const int *src[] = { &inst->detune_cents, &inst->stack, &inst->pwm_depth, &inst->pwm_rate,
                         &inst->vibrato_rate, &inst->vibrato_delay_ms, &inst->accent_gain,
                         &inst->pitch, &inst->pitch_time, &inst->noise, &inst->accents_marked };
    for (int i = 0; i < 11; i++) {
        if (!(voice->set & (1u << i)) && (inst->set & (1u << i))) {
            *dst[i] = *src[i];
            voice->set |= 1u << i;
        }
    }
}

static void parse_voice_directives(AbcVoice *v, const char *val)
{
    parse_extras(val, &v->extras);
    const char *instrument = strstr(val, "instrument=");
    const char *amp = strstr(val, "amp=");
    const char *wave = strstr(val, "wave=");
    const char *duty = strstr(val, "duty=");
    const char *attack = strstr(val, "attack=");
    const char *decay = strstr(val, "decay=");
    const char *sustain = strstr(val, "sustain=");
    const char *release = strstr(val, "release=");
    const char *gate = strstr(val, "gate=");
    const char *vibrato = strstr(val, "vibrato=");
    const char *glide = strstr(val, "glide=");
    const char *fx = strstr(val, "fx=");

    /* Check for instrument reference first */
    if (instrument) {
        sscanf(instrument + 11, "%31s", v->instrument_ref);
    }

    if (amp) {
        int a = atoi(amp + 4);
        if (validate_amplitude(a)) {
            v->amplitude = a;
        } else {
            fprintf(stderr, "Warning: amplitude %d out of range (0-127), using default\n", a);
        }
    }
    if (strstr(val, "staccato")) v->staccato = 1;
    if (wave) {
        char wname[16] = {0};
        sscanf(wave + 5, "%15s", wname);
        if (validate_waveform_name(wname)) {
            v->waveform = parse_waveform_name(wname);
        } else {
            fprintf(stderr, "Warning: unknown waveform '%s', using square\n", wname);
        }
    }
    if (duty) {
        int pct = atoi(duty + 5);
        if (validate_duty_cycle(pct)) {
            v->duty_cycle = pct;
        } else {
            fprintf(stderr, "Warning: duty cycle %d out of range (1-99), using default\n", pct);
        }
    }
    if (attack) v->attack_ms = atoi(attack + 7);
    if (decay) v->decay_ms = atoi(decay + 6);
    if (sustain) v->sustain_level = atoi(sustain + 8);
    if (release) v->release_ms = atoi(release + 8);
    if (gate) v->gate_percent = atoi(gate + 5);
    if (vibrato) {
        v->vibrato_cents = atoi(vibrato + 8);
        if (v->vibrato_rate <= 0) v->vibrato_rate = ABC_DEFAULT_VIBRATO_RATE;
    }
    if (glide) v->glide_ms = atoi(glide + 6);
    if (fx) {
        int fb = atoi(fx + 3);
        if (validate_fx_bus(fb)) {
            v->fx_bus = fb;
        }
    }

    if (v->sustain_level < 0) v->sustain_level = 0;
    if (v->sustain_level > 100) v->sustain_level = 100;
    if (v->gate_percent < 1) v->gate_percent = 1;
    if (v->gate_percent > 100) v->gate_percent = 100;
}

static int parse_int_param(const char *text, const char *key, int fallback)
{
    const char *p, *best_match;
    if (!text || !key) return fallback;
    
    /* Find the LAST occurrence of the key to avoid partial matches like "delay_mix=" when looking for "mix=" */
    best_match = NULL;
    p = text;
    while ((p = strstr(p, key)) != NULL) {
        /* Check if this is a valid match (not part of a longer identifier) */
        if (p == text || (!isalnum(*(p-1)) && *(p-1) != '_')) {
            best_match = p;
        }
        p++;
    }
    
    if (!best_match) return fallback;
    return atoi(best_match + strlen(key));
}

static void parse_effect_directive(AbcMusic *music, const char *val)
{
    if (!music || !val) return;
    while (*val == ' ') val++;
    if (strncmp(val, ABC_EFFECT_DELAY, sizeof(ABC_EFFECT_DELAY) - 1) == 0) {
        music->fx_delay_steps = parse_int_param(val, "time=", music->fx_delay_steps);
        music->fx_delay_feedback = parse_int_param(val, "feedback=", music->fx_delay_feedback);
        music->fx_delay_mix = parse_int_param(val, "mix=", music->fx_delay_mix);
        return;
    }
    if (strncmp(val, ABC_EFFECT_DRIVE, sizeof(ABC_EFFECT_DRIVE) - 1) == 0) {
        music->fx_drive_amount = parse_int_param(val, "amount=", music->fx_drive_amount);
        return;
    }
    if (strncmp(val, ABC_EFFECT_LOWPASS, sizeof(ABC_EFFECT_LOWPASS) - 1) == 0) {
        music->fx_lowpass_amount = parse_int_param(val, "amount=", music->fx_lowpass_amount);
        return;
    }
}

static void parse_sidechain_directive(AbcMusic *music, const char *val)
{
    if (!music || !val) return;
    music->fx_sidechain_amount = parse_int_param(val, "amount=", music->fx_sidechain_amount);
    music->fx_sidechain_release_ms = parse_int_param(val, "release=", music->fx_sidechain_release_ms);
}

static void parse_swing_directive(AbcMusic *music, const char *val)
{
    if (!music || !val) return;
    while (*val == ' ') val++;
    int swing = atoi(val);
    if (swing < 0) swing = 0;
    if (swing > 100) swing = 100;
    music->swing_pct = swing;
}

static void parse_instrument_directive(AbcMusic *music, const char *val)
{
    if (!music || !val || music->instrument_count >= ABC_MAX_INSTRUMENTS) return;
    while (*val == ' ') val++;
    
    AbcInstrument *inst = &music->instruments[music->instrument_count];
    memset(inst, 0, sizeof(*inst));
    
    /* Parse instrument name (first token) */
    sscanf(val, "%31s", inst->name);
    
    /* Set defaults */
    inst->amplitude = 40;
    inst->waveform = DSP_WAVE_SQUARE;
    inst->duty_cycle = 25;
    inst->sustain_level = 100;
    inst->gate_percent = 90;
    inst->vibrato_cents = 0;
    inst->glide_ms = 0;
    inst->fx_bus = 0;
    
    /* Parse parameters */
    const char *preset = strstr(val, "preset=");
    const char *amp = strstr(val, "amp=");
    const char *wave = strstr(val, "wave=");
    const char *duty = strstr(val, "duty=");
    const char *attack = strstr(val, "attack=");
    const char *decay = strstr(val, "decay=");
    const char *sustain = strstr(val, "sustain=");
    const char *release = strstr(val, "release=");
    const char *gate = strstr(val, "gate=");
    const char *vibrato = strstr(val, "vibrato=");
    const char *glide = strstr(val, "glide=");
    const char *fx = strstr(val, "fx=");
    
    if (preset) sscanf(preset + 7, "%31s", inst->preset);
    if (amp) {
        int a = atoi(amp + 4);
        if (validate_amplitude(a)) inst->amplitude = a;
    }
    if (wave) {
        char wname[16] = {0};
        sscanf(wave + 5, "%15s", wname);
        if (validate_waveform_name(wname))
            inst->waveform = parse_waveform_name(wname);
    }
    if (duty) {
        int pct = atoi(duty + 5);
        if (validate_duty_cycle(pct)) inst->duty_cycle = pct;
    }
    if (attack) inst->attack_ms = atoi(attack + 7);
    if (decay) inst->decay_ms = atoi(decay + 6);
    if (sustain) {
        int s = atoi(sustain + 8);
        if (s < 0) s = 0;
        if (s > 100) s = 100;
        inst->sustain_level = s;
    }
    if (release) inst->release_ms = atoi(release + 8);
    if (gate) {
        int g = atoi(gate + 5);
        if (g < 1) g = 1;
        if (g > 100) g = 100;
        inst->gate_percent = g;
    }
    if (vibrato) inst->vibrato_cents = atoi(vibrato + 8);
    if (glide) inst->glide_ms = atoi(glide + 6);
    if (fx) {
        int fb = atoi(fx + 3);
        if (validate_fx_bus(fb)) inst->fx_bus = fb;
    }
    parse_extras(val, &inst->extras);
    
    music->instrument_count++;
}

static void parse_pattern_directive(AbcMusic *music, const char *val)
{
    if (!music || !val || music->pattern_count >= ABC_MAX_PATTERNS) return;
    while (*val == ' ') val++;
    
    AbcPattern *pat = &music->patterns[music->pattern_count];
    memset(pat, 0, sizeof(*pat));
    
    /* Parse pattern name */
    sscanf(val, "%31s", pat->name);
    
    /* Parse length parameter */
    const char *length = strstr(val, "length=");
    if (length) {
        pat->length = atoi(length + 7);
        if (pat->length < 1) pat->length = 16;
        if (pat->length > ABC_MAX_NOTES) pat->length = ABC_MAX_NOTES;
    } else {
        pat->length = 16; /* default */
    }
    
    pat->defined = 1;
    music->pattern_count++;
}

static void parse_arrangement_directive(AbcMusic *music, const char *val)
{
    if (!music || !val) return;
    while (*val == ' ') val++;
    
    music->arrangement_length = 0;
    
    /* Parse space-separated pattern names */
    char buf[512];
    snprintf(buf, sizeof(buf), "%.*s", (int)(sizeof(buf) - 1), val);
    
    char *token = strtok(buf, " \t");
    while (token && music->arrangement_length < ABC_MAX_ARRANGEMENT) {
        snprintf(music->arrangement[music->arrangement_length],
                 sizeof(music->arrangement[0]), "%s", token);
        music->arrangement_length++;
        token = strtok(NULL, " \t");
    }
}

static void parse_numbered_effect_directive(AbcMusic *music, const char *val)
{
    if (!music || !val) return;
    while (*val == ' ') val++;
    
    /* Parse bus number */
    int bus_num = atoi(val);
    if (!validate_fx_bus(bus_num)) return;
    
    /* Skip past the bus number */
    while (*val && (*val == ' ' || isdigit(*val))) val++;
    
    /* Ensure we have this bus */
    if (bus_num >= music->fx_bus_count) {
        music->fx_bus_count = bus_num + 1;
    }
    
    AbcFxBus *bus = &music->fx_buses[bus_num];
    bus->enabled = 1;
    
    /* Parse parameters */
    bus->delay_steps = parse_int_param(val, "delay_steps=", bus->delay_steps);
    bus->delay_feedback = parse_int_param(val, "delay_feedback=", bus->delay_feedback);
    bus->delay_mix = parse_int_param(val, "delay_mix=", bus->delay_mix);
    bus->drive_amount = parse_int_param(val, "drive=", bus->drive_amount);
    bus->lowpass_amount = parse_int_param(val, "lowpass=", bus->lowpass_amount);
    bus->sidechain_amount = parse_int_param(val, "sidechain=", bus->sidechain_amount);
    bus->sidechain_release_ms = parse_int_param(val, "sidechain_release=", bus->sidechain_release_ms);
    bus->mix_percent = parse_int_param(val, "mix=", bus->mix_percent);
    bus->ladder_amount = parse_int_param(val, "ladder=", bus->ladder_amount);
    bus->ladder_cutoff = parse_int_param(val, "ladder_cutoff=", bus->ladder_cutoff);
    bus->ladder_resonance = parse_int_param(val, "ladder_resonance=", bus->ladder_resonance);
    if (strstr(val, "mode=insert")) bus->insert = 1;
    {
        const char *key = strstr(val, "sidechain_key=");
        if (key) sscanf(key + 14, "%31s", bus->sidechain_key);
    }
    
    /* Set default mix if not specified */
    if (bus->mix_percent == 0) bus->mix_percent = 100;
    
    /* Validate ranges */
    if (bus->delay_feedback < 0) bus->delay_feedback = 0;
    if (bus->delay_feedback > 100) bus->delay_feedback = 100;
    if (bus->delay_mix < 0) bus->delay_mix = 0;
    if (bus->delay_mix > 100) bus->delay_mix = 100;
    if (bus->mix_percent < 1) bus->mix_percent = 1;
    if (bus->mix_percent > 100) bus->mix_percent = 100;
}

/* ─── Parser state ───────────────────────────────────────────── */

typedef struct {
    int beats_per_bar;    /* M: numerator */
    int beat_unit;        /* M: denominator */
    int default_len_num;  /* L: numerator (e.g. 1) */
    int default_len_den;  /* L: denominator (e.g. 16) */
    int tempo_bpm;        /* Q: beats per minute */
    int key_sharps;       /* key signature accidentals (positive=sharps, negative=flats) */
    int key_accidentals[7]; /* per-note accidentals from key: -1=flat, 0=natural, 1=sharp */
} AbcHeader;

static void parse_key_sig(const char *key_str, AbcHeader *h)
{
    memset(h->key_accidentals, 0, sizeof(h->key_accidentals));

    /* Simple key parsing: major keys and Dm */
    char base = key_str[0];
    int is_minor = (strstr(key_str, "m") != NULL || strstr(key_str, "min") != NULL);

    /* Determine number of sharps/flats from key */
    /* Order of sharps: F C G D A E B */
    /* Order of flats:  B E A D G C F */
    static const int sharp_order[7] = { 3, 0, 4, 1, 5, 2, 6 }; /* F,C,G,D,A,E,B note indices */
    static const int flat_order[7]  = { 6, 2, 5, 1, 4, 0, 3 }; /* B,E,A,D,G,C,F */

    /* Map base note to number of sharps in major key (C=0, G=1, D=2...) */
    int nsharps = 0;
    switch (base) {
        case 'C': nsharps = 0; break;
        case 'G': nsharps = 1; break;
        case 'D': nsharps = 2; break;
        case 'A': nsharps = 3; break;
        case 'E': nsharps = 4; break;
        case 'B': nsharps = 5; break;
        case 'F':
            /* Check for F# */
            if (key_str[1] == '#') nsharps = 6;
            else nsharps = -1; /* F major = 1 flat */
            break;
        default: nsharps = 0;
    }

    /* Minor keys: relative minor is 3 semitones below major,
       which means 3 fewer sharps */
    if (is_minor) nsharps -= 3;

    /* Check for explicit flats/sharps after note name */
    if (key_str[1] == 'b' && key_str[1] != '\0') nsharps -= 1; /* e.g. Bb major */

    h->key_sharps = nsharps;

    if (nsharps > 0) {
        for (int i = 0; i < nsharps && i < 7; i++)
            h->key_accidentals[sharp_order[i]] = 1;
    } else if (nsharps < 0) {
        int nflats = -nsharps;
        for (int i = 0; i < nflats && i < 7; i++)
            h->key_accidentals[flat_order[i]] = -1;
    }
}

/* ─── Parse a single ABC note from text ──────────────────────── */

/*
 * Parse one note starting at *p. Returns the number of characters consumed.
 * Sets *freq to the note frequency and *steps to the number of default-length
 * units this note occupies.
 */
static int parse_note(const char *p, const AbcHeader *h,
                      double *freq, int *steps)
{
    const char *start = p;
    *freq = 0.0;
    *steps = 1;

    /* skip barlines, spaces, decorations */
    while (*p == '|' || *p == ':' || *p == ' ' || *p == '[' || *p == ']')
        p++;

    if (*p == '\0' || *p == '\n' || *p == '%') {
        *steps = 0;
        return (int)(p - start);
    }

    /* rest */
    if (*p == 'z' || *p == 'x') {
        p++;
        /* parse optional length multiplier */
        int mult = 0;
        while (*p >= '0' && *p <= '9') {
            mult = mult * 10 + (*p - '0');
            p++;
        }
        if (mult > 0) *steps = mult;
        *freq = 0.0;
        return (int)(p - start);
    }

    /* accidental */
    int accidental = -99; /* -99 = use key signature */
    if (*p == '^') { accidental = 1; p++; if (*p == '^') { accidental = 2; p++; } }
    else if (*p == '_') { accidental = -1; p++; if (*p == '_') { accidental = -2; p++; } }
    else if (*p == '=') { accidental = 0; p++; }

    /* note letter */
    char note_char = *p;
    int note_idx = note_char_to_index(note_char);
    if (note_idx < 0) {
        *steps = 0;
        return (int)(p - start);
    }

    /* ABC: uppercase C-B = octave 4 (C4-B4), lowercase c-b = octave 5 (C5-B5) */
    int octave = (note_char >= 'a' && note_char <= 'z') ? 5 : 4;
    p++;

    /* octave modifiers */
    while (*p == '\'') { octave++; p++; }
    while (*p == ',')  { octave--; p++; }

    /* compute MIDI note */
    int semitone = note_semitones[note_idx];
    if (accidental != -99) {
        semitone += accidental;
    } else {
        semitone += h->key_accidentals[note_idx];
    }
    int midi = (octave + 1) * 12 + semitone; /* C4 = (4+1)*12 + 0 = 60 */

    *freq = semitone_to_freq(midi);

    /* parse optional length multiplier */
    int mult = 0;
    while (*p >= '0' && *p <= '9') {
        mult = mult * 10 + (*p - '0');
        p++;
    }
    if (mult > 0) *steps = mult;

    /* handle fractional lengths (e.g. A/ = half, A/2 = half) */
    if (*p == '/') {
        p++;
        int div = 2;
        if (*p >= '0' && *p <= '9') {
            div = 0;
            while (*p >= '0' && *p <= '9') {
                div = div * 10 + (*p - '0');
                p++;
            }
        }
        /* fractional notes: round to at least 1 step */
        if (mult == 0) mult = 1;
        *steps = (mult > div) ? mult / div : 1;
    }

    return (int)(p - start);
}

/* ─── BRYX: decorations, chords and the one element reader ────── */

/* Append `steps` steps of a note (up to three pitches, 0 = none) to v,
 * with the voice's current dynamic and any pending accent. */
static void append_steps(AbcVoice *v, double f1, double f2, double f3, int steps)
{
    for (int s = 0; s < steps && v->note_count < ABC_MAX_NOTES; s++) {
        int n = v->note_count;
        v->freqs[n] = f1;
        v->freqs2[n] = f2;
        v->freqs3[n] = f3;
        v->onset[n] = (s == 0);
        v->vel[n] = (unsigned char)v->cur_vel;
        v->acc[n] = (unsigned char)(s == 0 && v->pending_accent && f1 > 0.0);
        v->note_count++;
    }
    if (steps > 0) v->pending_accent = 0;
}

/* !pp! .. !fff! set the voice's dynamic until the next mark; !>! accents
 * the next note. Other decorations are ignored, as before. */
static void apply_decoration(AbcVoice *v, const char *name, int len)
{
    static const struct { const char *mark; int velocity; } dynamics[] = {
        { "ppp", 24 }, { "pp", 40 }, { "p", 56 }, { "mp", 72 },
        { "mf", 88 }, { "f", 104 }, { "ff", 120 }, { "fff", 127 },
    };
    for (size_t i = 0; i < sizeof(dynamics) / sizeof(dynamics[0]); i++) {
        if ((int)strlen(dynamics[i].mark) == len && strncmp(name, dynamics[i].mark, (size_t)len) == 0) {
            v->cur_vel = dynamics[i].velocity;
            return;
        }
    }
    if ((len == 1 && name[0] == '>') || (len == 6 && strncmp(name, "accent", 6) == 0))
        v->pending_accent = 1;
}

/* A '[' opens a chord when a note follows, not an inline field like [V:. */
static int is_chord_start(const char *p)
{
    if (*p == '^' || *p == '_' || *p == '=') return 1;
    return note_char_to_index(*p) >= 0 && p[1] != ':';
}

/* Parse one element at p (a decoration, a chord, a note or a rest) and
 * append it to v. Returns the characters consumed; 0 means p is not an
 * element (the caller skips one character, as the original loop did). */
static int parse_element(const char *p, const AbcHeader *h, AbcVoice *v)
{
    const char *start = p;

    if (*p == '!') {
        const char *end = strchr(p + 1, '!');
        if (!end) return 1;
        apply_decoration(v, p + 1, (int)(end - p - 1));
        return (int)(end - p + 1);
    }
    if (*p == '[' && is_chord_start(p + 1)) {
        double f[3] = { 0.0, 0.0, 0.0 };
        int count = 0, steps = 1, mult = 0;
        p++;
        while (*p && *p != ']') {
            double freq;
            int st, used;
            if (!is_chord_start(p)) { p++; continue; }
            used = parse_note(p, h, &freq, &st);
            if (used <= 0) { p++; continue; }
            if (freq > 0.0 && count < 3) {
                if (count == 0) steps = st;
                f[count++] = freq;
            }
            p += used;
        }
        if (*p == ']') p++;
        while (*p >= '0' && *p <= '9') mult = mult * 10 + (*p++ - '0');
        if (mult > 0) steps *= mult;
        if (*p == '/') {
            int div = 0;
            p++;
            while (*p >= '0' && *p <= '9') div = div * 10 + (*p++ - '0');
            steps /= div > 0 ? div : 2;
            if (steps < 1) steps = 1;
        }
        append_steps(v, f[0], f[1], f[2], steps);
        return (int)(p - start);
    }
    {
        double freq;
        int steps;
        int consumed = parse_note(p, h, &freq, &steps);
        if (consumed > 0)
            append_steps(v, freq, 0.0, 0.0, steps);
        return consumed;
    }
}

/* ─── Public API ─────────────────────────────────────────────── */

int abc_load(const char *path, AbcMusic *music)
{
    FILE *f = fopen(path, "r");
    if (!f) return -1;

    memset(music, 0, sizeof(*music));
    music->voice_count = 0;
    music->bpm = 120;
    music->swing_pct = 0;
    music->fx_sidechain_release_ms = ABC_DEFAULT_SIDECHAIN_RELEASE_MS;
    music->instrument_count = 0;
    music->pattern_count = 0;
    music->arrangement_length = 0;
    music->fx_bus_count = ABC_DEFAULT_FX_BUS_COUNT; /* at least one FX bus for backward compat */
    
    /* Initialize default FX bus (bus 0) */
    memset(&music->fx_buses[0], 0, sizeof(AbcFxBus));
    music->fx_buses[0].enabled = 1;
    music->fx_buses[0].mix_percent = 100;

    AbcHeader header;
    memset(&header, 0, sizeof(header));
    header.beats_per_bar = 4;
    header.beat_unit = 4;
    header.default_len_num = 1;
    header.default_len_den = 16;
    header.tempo_bpm = 120;

    int current_voice = -1;
    int in_repeat = 0;
    char repeat_buf[4096] = {0};
    int repeat_len = 0;

    char line[1024];
    while (fgets(line, sizeof(line), f)) {
        /* strip trailing newline */
        int len = (int)strlen(line);
        while (len > 0 && (line[len-1] == '\n' || line[len-1] == '\r'))
            line[--len] = '\0';

        /* skip comments */
        if (line[0] == '%' && line[1] != '%') continue;

        /* header fields */
        if (len >= 2 && line[1] == ':' && line[0] != '|') {
            char field = line[0];
            const char *val = line + 2;
            while (*val == ' ') val++;

            switch (field) {
                case 'X': break; /* reference number, ignored */
                case 'T': /* title */
                    snprintf(music->title, sizeof(music->title), "%.127s", val);
                    break;
                case 'M': /* meter */
                    sscanf(val, "%d/%d", &header.beats_per_bar, &header.beat_unit);
                    break;
                case 'L': /* default note length */
                    sscanf(val, "%d/%d", &header.default_len_num, &header.default_len_den);
                    break;
                case 'Q': { /* tempo */
                    int bpm = 120;
                    /* Q:1/4=120 or Q:120 */
                    const char *eq = strchr(val, '=');
                    if (eq) bpm = atoi(eq + 1);
                    else bpm = atoi(val);
                    if (bpm > 0) {
                        header.tempo_bpm = bpm;
                        music->bpm = bpm;
                    }
                    break;
                }
                case 'K': /* key */
                    parse_key_sig(val, &header);
                    break;
                case 'V': { /* voice */
                    /* Find or create voice by name */
                    char vname[32] = {0};
                    sscanf(val, "%31s", vname);

                    /* look for existing voice */
                    int found = -1;
                    for (int i = 0; i < music->voice_count; i++) {
                        if (strcmp(music->voices[i].name, vname) == 0) {
                            found = i;
                            break;
                        }
                    }

                    if (found >= 0) {
                        current_voice = found;
                    } else if (music->voice_count < ABC_MAX_VOICES) {
                        current_voice = music->voice_count++;
                        AbcVoice *v = &music->voices[current_voice];
                        memset(v, 0, sizeof(*v));
                        snprintf(v->name, sizeof(v->name), "%s", vname);
                        v->amplitude = 40; /* default */
                        v->staccato = 0;
                        v->waveform = DSP_WAVE_SQUARE;
                        v->duty_cycle = 25;
                        v->attack_ms = 0;
                        v->decay_ms = 0;
                        v->sustain_level = 100;
                        v->release_ms = 0;
                        v->gate_percent = 90;
                        v->vibrato_cents = 0;
                        v->vibrato_rate = ABC_DEFAULT_VIBRATO_RATE;
                        v->glide_ms = 0;
                    }

                    /* parse voice directives: amp=N, staccato */
                    AbcVoice *v = &music->voices[current_voice];
                    parse_voice_directives(v, val);
                    break;
                }
                default: break;
            }
            continue;
        }

        /* %%voice directive (pseudo-comment) */
        if (strncmp(line, "%%voice", 7) == 0) {
            const char *val = line + 7;
            while (*val == ' ') val++;

            char vname[32] = {0};
            sscanf(val, "%31s", vname);

            if (music->voice_count < ABC_MAX_VOICES) {
                current_voice = music->voice_count++;
                AbcVoice *v = &music->voices[current_voice];
                memset(v, 0, sizeof(*v));
                snprintf(v->name, sizeof(v->name), "%s", vname);
                v->amplitude = 40;
                v->staccato = 0;
                v->waveform = DSP_WAVE_SQUARE;
                v->duty_cycle = 25;
                v->attack_ms = 0;
                v->decay_ms = 0;
                v->sustain_level = 100;
                v->release_ms = 0;
                v->gate_percent = 90;
                v->vibrato_cents = 0;
                v->vibrato_rate = ABC_DEFAULT_VIBRATO_RATE;
                v->glide_ms = 0;
                v->fx_bus = 0;
                v->instrument_ref[0] = '\0';

                parse_voice_directives(v, val);
            }
            continue;
        }
        
        /* %%instrument directive */
        if (strncmp(line, "%%instrument", 12) == 0) {
            parse_instrument_directive(music, line + 12);
            continue;
        }
        
        /* %%pattern directive */
        if (strncmp(line, "%%pattern", 9) == 0) {
            parse_pattern_directive(music, line + 9);
            continue;
        }
        
        /* %%arrangement directive */
        if (strncmp(line, "%%arrangement", 13) == 0) {
            parse_arrangement_directive(music, line + 13);
            continue;
        }
        
        /* %%effect directive - check if numbered (e.g., "%%effect 0") */
        if (strncmp(line, "%%effect", 8) == 0) {
            const char *val = line + 8;
            while (*val == ' ') val++;
            
            /* Check if first char after spaces is a digit */
            if (isdigit(*val)) {
                parse_numbered_effect_directive(music, val);
            } else {
                /* Legacy single FX bus - map to bus 0 */
                parse_effect_directive(music, val);
            }
            continue;
        }
        
        if (strncmp(line, "%%sidechain", 11) == 0) {
            parse_sidechain_directive(music, line + 11);
            continue;
        }
        if (strncmp(line, "%%swing", 7) == 0) {
            parse_swing_directive(music, line + 7);
            continue;
        }
        /* BRYX: %%hold on, and %%master <percent of full gain> */
        if (strncmp(line, "%%hold", 6) == 0) {
            music->hold = strstr(line + 6, "on") != NULL;
            continue;
        }
        if (strncmp(line, "%%ramp", 6) == 0) {
            AbcRamp *r = &music->ramps[music->ramp_count];
            if (music->ramp_count < SEQ_MAX_RAMPS &&
                sscanf(line + 6, "%31s %23s %d %d %d %d", r->target, r->param,
                       &r->from, &r->to, &r->start_step, &r->end_step) == 6)
                music->ramp_count++;
            else
                fprintf(stderr, "Warning: ignored %%%%ramp line: %s\n", line);
            continue;
        }
        if (strncmp(line, "%%master", 8) == 0) {
            double pct = atof(line + 8);
            music->master_permille = pct > 0.0 ? (int)(pct * 10.0 + 0.5) : 0;
            continue;
        }

        /* skip other %% directives */
        if (line[0] == '%') continue;

        /* If no voice defined yet, create a default one */
        if (current_voice < 0) {
            if (music->voice_count < ABC_MAX_VOICES) {
                current_voice = music->voice_count++;
                AbcVoice *v = &music->voices[current_voice];
                memset(v, 0, sizeof(*v));
                snprintf(v->name, sizeof(v->name), "default");
                v->amplitude = 40;
                v->waveform = DSP_WAVE_SQUARE;
                v->duty_cycle = 25;
                v->attack_ms = 0;
                v->decay_ms = 0;
                v->sustain_level = 100;
                v->release_ms = 0;
                v->gate_percent = 90;
                v->vibrato_cents = 0;
                v->vibrato_rate = ABC_DEFAULT_VIBRATO_RATE;
                v->glide_ms = 0;
                v->fx_bus = 0;
                v->instrument_ref[0] = '\0';
            } else {
                current_voice = 0;
            }
        }

        AbcVoice *v = &music->voices[current_voice];

        /* parse note data from this line */
        const char *p = line;
        while (*p) {
            /* skip whitespace and barlines */
            while (*p == ' ' || *p == '\t') p++;
            if (*p == '\0' || *p == '%') break;

            /* handle repeat start */
            if (*p == '|' && *(p+1) == ':') {
                in_repeat = 1;
                repeat_len = 0;
                repeat_buf[0] = '\0';
                p += 2;
                continue;
            }

            /* handle repeat end */
            if (*p == ':' && *(p+1) == '|') {
                if (in_repeat && repeat_len > 0) {
                    /* replay the repeated section */
                    const char *rp = repeat_buf;
                    while (*rp) {
                        int consumed = parse_element(rp, &header, v);
                        if (consumed == 0) { rp++; continue; }
                        rp += consumed;
                    }
                }
                in_repeat = 0;
                p += 2;
                continue;
            }

            /* handle simple barline */
            if (*p == '|') { p++; continue; }

            int consumed = parse_element(p, &header, v);
            if (consumed == 0) { p++; continue; }

            /* if in repeat section, also buffer the raw text */
            if (in_repeat) {
                int avail = (int)sizeof(repeat_buf) - repeat_len - 1;
                if (consumed < avail) {
                    memcpy(repeat_buf + repeat_len, p, consumed);
                    repeat_len += consumed;
                    repeat_buf[repeat_len++] = ' ';
                    repeat_buf[repeat_len] = '\0';
                }
            }

            p += consumed;
        }
    }

    fclose(f);

    /* compute step duration from tempo and default note length */
    /* default length in beats = default_len_num / default_len_den * beat_unit */
    /* step_ms = (60000 / bpm) * (default_len / quarter_note) */
    double quarter_ms = 60000.0 / music->bpm;
    double note_fraction = (double)header.default_len_num / header.default_len_den;
    /* note_fraction of a whole note; a quarter note = 1/4 */
    music->step_ms = (int)(quarter_ms * note_fraction * 4.0);
    if (music->step_ms < 10) music->step_ms = 125; /* fallback */

    /* Apply instrument presets to voices that reference them */
    for (int v = 0; v < music->voice_count; v++) {
        AbcVoice *voice = &music->voices[v];
        if (voice->instrument_ref[0] != '\0') {
            /* Find matching instrument */
            for (int i = 0; i < music->instrument_count; i++) {
                AbcInstrument *inst = &music->instruments[i];
                if (strcmp(voice->instrument_ref, inst->name) == 0) {
                    /* Apply instrument settings to voice (voice settings override) */
                    if (voice->amplitude == 40) voice->amplitude = inst->amplitude;
                    if (voice->waveform == DSP_WAVE_SQUARE) voice->waveform = inst->waveform;
                    if (voice->duty_cycle == 25) voice->duty_cycle = inst->duty_cycle;
                    if (voice->attack_ms == 0) voice->attack_ms = inst->attack_ms;
                    if (voice->decay_ms == 0) voice->decay_ms = inst->decay_ms;
                    if (voice->sustain_level == 100) voice->sustain_level = inst->sustain_level;
                    if (voice->release_ms == 0) voice->release_ms = inst->release_ms;
                    if (voice->gate_percent == 90) voice->gate_percent = inst->gate_percent;
                    if (voice->vibrato_cents == 0) voice->vibrato_cents = inst->vibrato_cents;
                    if (voice->glide_ms == 0) voice->glide_ms = inst->glide_ms;
                    if (voice->fx_bus == 0) voice->fx_bus = inst->fx_bus;
                    merge_extras(&voice->extras, &inst->extras);
                    break;
                }
            }
        }
    }

    /* Map legacy FX fields to bus 0 for backward compatibility */
    if (music->fx_delay_steps > 0 || music->fx_drive_amount > 0 || music->fx_lowpass_amount > 0 ||
        music->fx_sidechain_amount > 0) {
        music->fx_buses[0].delay_steps = music->fx_delay_steps;
        music->fx_buses[0].delay_feedback = music->fx_delay_feedback;
        music->fx_buses[0].delay_mix = music->fx_delay_mix;
        music->fx_buses[0].drive_amount = music->fx_drive_amount;
        music->fx_buses[0].lowpass_amount = music->fx_lowpass_amount;
        music->fx_buses[0].sidechain_amount = music->fx_sidechain_amount;
        music->fx_buses[0].sidechain_release_ms = music->fx_sidechain_release_ms;
    }

    return 0;
}

int abc_load_voices(const char *paths[], int path_count, AbcMusic *music)
{
    memset(music, 0, sizeof(*music));
    music->bpm = 120;
    music->step_ms = 125;
    music->fx_sidechain_release_ms = ABC_DEFAULT_SIDECHAIN_RELEASE_MS;
    music->fx_bus_count = ABC_DEFAULT_FX_BUS_COUNT;

    for (int i = 0; i < path_count && i < ABC_MAX_VOICES; i++) {
        AbcMusic single;
        if (abc_load(paths[i], &single) != 0) continue;

        /* copy first voice from each file */
        if (single.voice_count > 0 && music->voice_count < ABC_MAX_VOICES) {
            music->voices[music->voice_count] = single.voices[0];
            music->voice_count++;
        }

        /* use tempo from first file */
        if (i == 0) {
            music->bpm = single.bpm;
            music->step_ms = single.step_ms;
            music->swing_pct = single.swing_pct;
            music->instrument_count = single.instrument_count;
            memcpy(music->instruments, single.instruments, sizeof(music->instruments));
            music->pattern_count = single.pattern_count;
            memcpy(music->patterns, single.patterns, sizeof(music->patterns));
            music->arrangement_length = single.arrangement_length;
            memcpy(music->arrangement, single.arrangement, sizeof(music->arrangement));
            music->fx_bus_count = single.fx_bus_count;
            memcpy(music->fx_buses, single.fx_buses, sizeof(music->fx_buses));
            music->fx_delay_steps = single.fx_delay_steps;
            music->fx_delay_feedback = single.fx_delay_feedback;
            music->fx_delay_mix = single.fx_delay_mix;
            music->fx_drive_amount = single.fx_drive_amount;
            music->fx_lowpass_amount = single.fx_lowpass_amount;
            music->fx_sidechain_amount = single.fx_sidechain_amount;
            music->fx_sidechain_release_ms = single.fx_sidechain_release_ms;
            snprintf(music->title, sizeof(music->title), "%s", single.title);
        }
    }

    return (music->voice_count > 0) ? 0 : -1;
}

int abc_build_seq_song(const AbcMusic *music, SeqSong *song)
{
    int track_count;
    int max_steps = 0;
    int cursor = 0;
    int arrangement_written = 0;

    if (!music || !song || music->voice_count <= 0)
        return -1;

    memset(song, 0, sizeof(*song));
    track_count = music->voice_count;
    if (track_count > SEQ_MAX_TRACKS) track_count = SEQ_MAX_TRACKS;
    if (track_count <= 0) return -1;

    snprintf(song->title, sizeof(song->title), "%.127s", music->title);
    song->tempo_bpm = music->bpm > 0 ? music->bpm : 120;
    song->steps_per_beat = default_steps_per_beat(song->tempo_bpm, music->step_ms);
    if (music->swing_pct <= 0) song->swing_pct = 50;
    else if (music->swing_pct < 50) song->swing_pct = 50;
    else if (music->swing_pct > 75) song->swing_pct = 75;
    else song->swing_pct = music->swing_pct;

    song->instrument_count = track_count;
    for (int t = 0; t < track_count; t++) {
        const AbcVoice *v = &music->voices[t];
        SeqInstrument *inst = &song->instruments[t];
        memset(inst, 0, sizeof(*inst));
        inst->waveform = sanitize_waveform(v->waveform);
        /* Keep ABC-to-sequencer conversion gain conservative to avoid hard saturation. */
        inst->amplitude = dsp_clampi((v->amplitude * ABC_TO_SEQ_AMP_PCT) / 100, 0, 127);
        inst->pulse_width = dsp_clampi(v->duty_cycle > 0 ? v->duty_cycle : 25, 1, 99);
        inst->envelope.attack_ms = v->attack_ms;
        inst->envelope.decay_ms = v->decay_ms;
        inst->envelope.sustain_level = dsp_clampi(v->sustain_level, 0, 100);
        inst->envelope.release_ms = v->release_ms;
        inst->envelope.gate_percent = dsp_clampi(v->gate_percent > 0 ? v->gate_percent : (v->staccato ? 75 : 90), 1, 100);
        inst->vibrato_depth_cents = dsp_clampi(v->vibrato_cents, 0, 100);
        inst->vibrato_rate = v->vibrato_rate > 0 ? v->vibrato_rate : ABC_DEFAULT_VIBRATO_RATE;
        inst->glide_ms = dsp_clampi(v->glide_ms, 0, 500);
        inst->fx_send = dsp_clampi(v->fx_bus, 0, SEQ_MAX_FX_BUSES - 1);
        inst->accent_gain = 20;
        {
            /* BRYX: the extras ABC can now set. */
            const AbcExtras *x = &v->extras;
            if (x->set & (1u << 0)) inst->detune_cents = dsp_clampi(x->detune_cents, 0, 200);
            if (x->set & (1u << 1)) inst->stacked_voices = dsp_clampi(x->stack, 1, 2);
            if (x->set & (1u << 2)) inst->pwm_depth = dsp_clampi(x->pwm_depth, 0, 40);
            if (x->set & (1u << 3)) inst->pwm_rate = dsp_clampi(x->pwm_rate, 0, 20000);
            if (x->set & (1u << 4)) inst->vibrato_rate = dsp_clampi(x->vibrato_rate, 1, 20000);
            if (x->set & (1u << 5)) inst->vibrato_delay_ms = dsp_clampi(x->vibrato_delay_ms, 0, 2000);
            if (x->set & (1u << 6)) inst->accent_gain = dsp_clampi(x->accent_gain, 0, 100);
            if (x->set & (1u << 7)) inst->pitch_env_cents = dsp_clampi(x->pitch, -48, 48) * 100;
            inst->pitch_env_ms = (x->set & (1u << 8)) ? dsp_clampi(x->pitch_time, 1, 2000) : 40;
            if (x->set & (1u << 9)) inst->noise_clock = dsp_clampi(x->noise, 0, 2);
        }
        if (inst->waveform == DSP_WAVE_NOISE)
            inst->noise_mode = 1;
    }

    song->fx_bus_count = music->fx_bus_count;
    if (song->fx_bus_count < 1) song->fx_bus_count = ABC_DEFAULT_FX_BUS_COUNT;
    if (song->fx_bus_count > SEQ_MAX_FX_BUSES) song->fx_bus_count = SEQ_MAX_FX_BUSES;
    for (int i = 0; i < song->fx_bus_count; i++) {
        const AbcFxBus *src = &music->fx_buses[i];
        SeqFxBus *dst = &song->fx_buses[i];
        dst->enabled = src->enabled ? 1 : 0;
        dst->delay_steps = dsp_clampi(src->delay_steps, 0, 64);
        dst->delay_feedback = dsp_clampi(src->delay_feedback, 0, 95);
        dst->delay_mix = dsp_clampi(src->delay_mix, 0, 100);
        dst->drive_amount = dsp_clampi(src->drive_amount, 0, 100);
        dst->lowpass_amount = dsp_clampi(src->lowpass_amount, 0, 100);
        dst->sidechain_amount = dsp_clampi(src->sidechain_amount, 0, 100);
        dst->sidechain_release_ms = dsp_clampi(src->sidechain_release_ms > 0 ? src->sidechain_release_ms : ABC_DEFAULT_SIDECHAIN_RELEASE_MS, 10, 2000);
        dst->mix_percent = dsp_clampi(src->mix_percent > 0 ? src->mix_percent : 100, 0, 100);
        dst->ladder_amount = dsp_clampi(src->ladder_amount, 0, 100);
        dst->ladder_cutoff = dsp_clampi(src->ladder_cutoff > 0 ? src->ladder_cutoff : 50, 1, 100);
        dst->ladder_resonance = dsp_clampi(src->ladder_resonance, 0, 100);
        /* BRYX: insert mode, and a sidechain keyed by a voice's name or its
         * instrument's. */
        dst->insert = src->insert ? 1 : 0;
        dst->sidechain_key = 0;
        if (src->sidechain_key[0]) {
            for (int t = 0; t < track_count; t++) {
                if (strcmp(music->voices[t].name, src->sidechain_key) == 0 ||
                    strcmp(music->voices[t].instrument_ref, src->sidechain_key) == 0) {
                    dst->sidechain_key = t + 1;
                    break;
                }
            }
        }
    }
    song->master_permille = music->master_permille;
    for (int t = 0; t < track_count && !song->chord_voices; t++)
        for (int n = 0; n < music->voices[t].note_count; n++)
            if (music->voices[t].freqs2[n] > 0.0) {
                song->chord_voices = 1;
                break;
            }

    /* BRYX: resolve each %%ramp to its bus or track and parameter. */
    song->ramp_count = 0;
    for (int i = 0; i < music->ramp_count; i++) {
        static const struct { const char *name; SeqRampParam param; } params[] = {
            { "ladder_cutoff", SEQ_RAMP_LADDER_CUTOFF }, { "ladder_resonance", SEQ_RAMP_LADDER_RESONANCE },
            { "lowpass", SEQ_RAMP_LOWPASS }, { "delay_mix", SEQ_RAMP_DELAY_MIX },
            { "drive", SEQ_RAMP_DRIVE }, { "mix", SEQ_RAMP_BUS_MIX }, { "duty", SEQ_RAMP_DUTY },
        };
        const AbcRamp *src = &music->ramps[i];
        SeqRamp *dst = &song->ramps[song->ramp_count];
        int found = 0;
        memset(dst, 0, sizeof(*dst));
        dst->track = dst->bus = -1;
        for (size_t k = 0; k < sizeof(params) / sizeof(params[0]); k++) {
            if (strcmp(src->param, params[k].name) == 0) {
                dst->param = params[k].param;
                found = 1;
            }
        }
        if (strncmp(src->target, "bus", 3) == 0 && dst->param != SEQ_RAMP_DUTY) {
            dst->bus = atoi(src->target + 3);
        } else if (dst->param == SEQ_RAMP_DUTY) {
            for (int t = 0; t < track_count; t++) {
                if (strcmp(music->voices[t].name, src->target) == 0 ||
                    strcmp(music->voices[t].instrument_ref, src->target) == 0) {
                    dst->track = t;
                    break;
                }
            }
        }
        if (!found || (dst->bus < 0 && dst->track < 0)) {
            fprintf(stderr, "Warning: ignored ramp of %s on %s\n", src->param, src->target);
            continue;
        }
        dst->from = src->from;
        dst->to = src->to;
        dst->start_step = src->start_step;
        dst->end_step = src->end_step < src->start_step ? src->start_step : src->end_step;
        song->ramp_count++;
    }

    for (int t = 0; t < track_count; t++) {
        if (music->voices[t].note_count > max_steps)
            max_steps = music->voices[t].note_count;
    }
    if (max_steps <= 0) return -1;

    if (music->pattern_count > 0 && music->arrangement_length > 0) {
        for (int i = 0; i < music->arrangement_length; i++) {
            int found = -1;
            int length = 16;

            for (int p = 0; p < music->pattern_count; p++) {
                if (strcmp(music->arrangement[i], music->patterns[p].name) == 0) {
                    found = p;
                    length = music->patterns[p].length;
                    break;
                }
            }
            if (found < 0)
                return -1;
            if (append_pattern_instance(song, music, track_count, length, cursor, &arrangement_written) != 0)
                return -1;
            cursor += dsp_clampi(length, 1, SEQ_MAX_STEPS);
        }
    } else if (music->pattern_count > 0) {
        for (int p = 0; p < music->pattern_count; p++) {
            int length = music->patterns[p].length > 0 ? music->patterns[p].length : 16;

            if (append_pattern_instance(song, music, track_count, length, cursor, &arrangement_written) != 0)
                return -1;
            cursor += dsp_clampi(length, 1, SEQ_MAX_STEPS);
        }
    } else {
        int pattern_count = (max_steps + SEQ_MAX_STEPS - 1) / SEQ_MAX_STEPS;
        if (pattern_count < 1) pattern_count = 1;
        if (pattern_count > SEQ_MAX_PATTERNS) pattern_count = SEQ_MAX_PATTERNS;
        for (int p = 0; p < pattern_count; p++) {
            int base_step = p * SEQ_MAX_STEPS;
            int length = max_steps - base_step;
            if (length > SEQ_MAX_STEPS) length = SEQ_MAX_STEPS;
            if (length < 1) length = 1;
            if (append_pattern_instance(song, music, track_count, length, base_step, &arrangement_written) != 0)
                return -1;
        }
    }

    if (arrangement_written == 0) {
        if (song->pattern_count <= 0) return -1;
        song->arrangement[0] = 0;
        arrangement_written = 1;
    }
    song->arrangement_length = arrangement_written;
    return 0;
}

unsigned char *abc_generate_pcm(const AbcMusic *music, int *out_len)
{
    SeqSong song;

    if (!out_len) return NULL;
    *out_len = 0;
    if (abc_build_seq_song(music, &song) != 0)
        return NULL;
    return audio_mix_render_song(&song, SAMPLE_RATE_ABC, out_len);
}
