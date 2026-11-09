#ifndef ELECTRIC_PULSE_H
#define ELECTRIC_PULSE_H

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <ctype.h>
#include <stdint.h>
#include <time.h>

#include "audio_seq.h"
#include "audio_engine.h"

/* abc.c — ABC notation parser and PCM generator */
#define ABC_MAX_VOICES     8
#define ABC_MAX_NOTES      1024
#define ABC_MAX_INSTRUMENTS 8
#define ABC_MAX_PATTERNS   16
#define ABC_MAX_ARRANGEMENT 32
#define ABC_MAX_FX_BUSES   4
#define SAMPLE_RATE_ABC    22050

/* BRYX: instrument parameters the mixer always had but ABC could not
 * reach, plus a vibrato fade-in. `set` has bit i on when field i was
 * written, so a voice can tell "not given" from zero. */
typedef struct {
    int detune_cents;     /* bit 0: detune= (spread of a stacked pair, cents) */
    int stack;            /* bit 1: stack= (2 = two detuned oscillators) */
    int pwm_depth;        /* bit 2: pwm= (pulse-width sweep depth, duty percent) */
    int pwm_rate;         /* bit 3: pwm_rate= (millihertz) */
    int vibrato_rate;     /* bit 4: vibrato_rate= (millihertz) */
    int vibrato_delay_ms; /* bit 5: vibrato_delay= (ms for vibrato to fade in) */
    int accent_gain;      /* bit 6: accent= (percent added on accented steps) */
    unsigned set;
} AbcExtras;

/* Named instrument preset */
typedef struct {
    char name[32];        /* instrument name */
    char preset[32];      /* preset identifier */
    int amplitude;
    int waveform;
    int duty_cycle;
    int attack_ms;
    int decay_ms;
    int sustain_level;
    int release_ms;
    int gate_percent;
    int vibrato_cents;
    int glide_ms;
    int fx_bus;          /* which FX bus (0-3) */
    AbcExtras extras;    /* BRYX */
} AbcInstrument;

/* Pattern definition */
typedef struct {
    char name[32];       /* pattern name (A, B, C, etc.) */
    int length;          /* pattern length in steps */
    int defined;         /* 1 if pattern has been defined */
} AbcPattern;

/* FX bus configuration */
typedef struct {
    int enabled;
    int delay_steps;
    int delay_feedback;
    int delay_mix;
    int drive_amount;
    int lowpass_amount;
    int sidechain_amount;
    int sidechain_release_ms;
    int mix_percent;     /* bus output mix (0-100) */
    int ladder_amount;   /* moog ladder wet mix 0-100; 0 = disabled */
    int ladder_cutoff;   /* 1-100 (percent of Nyquist) */
    int ladder_resonance;/* 0-100 */
    int insert;          /* BRYX: mode=insert */
    char sidechain_key[32]; /* BRYX: sidechain_key=<voice or instrument name> */
} AbcFxBus;

typedef struct {
    char name[32];
    char instrument_ref[32]; /* reference to instrument name, if any */
    int amplitude;        /* per-voice amplitude (0-127) */
    int staccato;         /* 1 = staccato (3/4 length), 0 = legato (9/10 length) */
    int waveform;         /* 0=square, 1=pulse, 2=triangle, 3=noise */
    int duty_cycle;       /* pulse duty cycle percent (1-99), ignored otherwise */
    int attack_ms;
    int decay_ms;
    int sustain_level;
    int release_ms;
    int gate_percent;
    int vibrato_cents;
    int vibrato_rate;
    int glide_ms;
    int fx_bus;           /* which FX bus to use (0-3) */
    double freqs[ABC_MAX_NOTES]; /* frequency per step (0 = rest) */
    unsigned char onset[ABC_MAX_NOTES]; /* BRYX: 1 where a written note begins */
    AbcExtras extras;     /* BRYX */
    int note_count;
} AbcVoice;

typedef struct {
    char title[128];
    int bpm;
    int step_ms;          /* duration of one default-length note in ms */
    int swing_pct;        /* swing percentage 0..100 (%%swing) */

    /* Instruments */
    AbcInstrument instruments[ABC_MAX_INSTRUMENTS];
    int instrument_count;

    /* Patterns */
    AbcPattern patterns[ABC_MAX_PATTERNS];
    int pattern_count;

    /* Arrangement */
    char arrangement[ABC_MAX_ARRANGEMENT][32]; /* pattern sequence */
    int arrangement_length;

    /* FX Buses (backward compatible: bus 0 is default) */
    AbcFxBus fx_buses[ABC_MAX_FX_BUSES];
    int fx_bus_count;

    /* Legacy FX fields (mapped to fx_buses[0] for compatibility) */
    int fx_delay_steps;
    int fx_delay_feedback;
    int fx_delay_mix;
    int fx_drive_amount;
    int fx_lowpass_amount;
    int fx_sidechain_amount;
    int fx_sidechain_release_ms;

    AbcVoice voices[ABC_MAX_VOICES];
    int voice_count;

    int hold;             /* BRYX: %%hold on: a note sounds once for its written length */
    int master_permille;  /* BRYX: %%master <percent>: output gain; 0 = unity */
} AbcMusic;

int abc_load(const char *path, AbcMusic *music);
int abc_load_voices(const char *paths[], int path_count, AbcMusic *music);
int abc_build_seq_song(const AbcMusic *music, SeqSong *out_song);
unsigned char *abc_generate_pcm(const AbcMusic *music, int *out_len);

#endif
