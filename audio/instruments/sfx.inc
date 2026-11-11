# The gameplay effects' instruments and desk (audio/score/sfx/*.score).
# Effects are folded to mono for the mu-law effects path and pitched by
# playback rate in the game, so everything here is centred, short and
# close: no seating, short releases, a small room only where a sound asks
# for one. Built from the score's own parts: the orchestra's samples and
# the studio synth.

aux sfx_room reverb preset=room gain=-4
aux sfx_echo delay time=0.25 time_r=0.25 feedback=0.3 lowcut=400 highcut=6000 gain=-6
bus sfx gain=0 hpf=40
master limit=-1

# Orchestra: plucks, taps and short notes.
inst sfx_pizz_hi  sfz=../samples/violins_pizz.sfz  bus=sfx pan=0 release=0.08
inst sfx_pizz_lo  sfz=../samples/celli_pizz.sfz    bus=sfx pan=0 release=0.08
inst sfx_spic     sfz=../samples/violins_spic.sfz  bus=sfx pan=0 release=0.05 send=sfx_echo:-10
inst sfx_trem     sfz=../samples/violins_trem.sfz  bus=sfx pan=0 attack=0.15 release=0.2 send=sfx_echo:-6
inst sfx_trumpet  sfz=../samples/trumpet_sus.sfz   bus=sfx pan=0 attack=0.01 release=0.25 send=sfx_room:-8
inst sfx_trumpet_stac sfz=../samples/trumpet_stac.sfz bus=sfx pan=0 release=0.06
inst sfx_trombone_stac sfz=../samples/trombone_stac.sfz bus=sfx pan=0 release=0.06
inst sfx_timp     sfz=../samples/timpani.sfz        bus=sfx pan=0 release=0.25
inst sfx_timp_roll sfz=../samples/timpani_roll.sfz  bus=sfx pan=0 attack=0.08 release=0.15
inst sfx_kit      sfz=../samples/kit.sfz            bus=sfx pan=0 release=0.3 send=sfx_room:-10

# Synth: the knocks, pops, zaps and air. The ones that carry an effect's
# body are driven: as dense for their peak as the chip effects they
# replace, so they hold their loudness without leaning on the limiter.
inst sfx_knock synth bus=sfx wave=triangle pitchenv=5 pitchtime=0.02 attack=0 decay=0.15 sustain=0 release=0.05 drive=0.6
inst sfx_pop   synth bus=sfx wave=pulse pw=0.22 pitchenv=7 pitchtime=0.014 filter=lp cutoff=6000 keytrack=0.5 attack=0 decay=0.3 sustain=0 release=0.06 drive=0.8
inst sfx_click synth bus=sfx wave=noise filter=hp cutoff=4000 attack=0 decay=0.008 sustain=0 release=0.006
inst sfx_glint synth bus=sfx wave=noise filter=hp cutoff=7000 res=0.3 attack=0 decay=0.018 sustain=0 release=0.014
inst sfx_tink  synth bus=sfx wave=triangle attack=0 decay=0.08 sustain=0 release=0.05 drive=0.6
inst sfx_ring  synth bus=sfx wave=pulse pw=0.4 filter=lp cutoff=5000 attack=0 decay=0.14 sustain=0 release=0.16 drive=0.6
inst sfx_bell  synth bus=sfx wave=triangle stack=2 detune=4 attack=0.001 decay=0.25 sustain=0 release=0.2 send=sfx_echo:-8
inst sfx_sour  synth bus=sfx wave=pulse pw=0.45 attack=0 decay=0.05 sustain=0.4 release=0.06 lfo1=sine:30:0 lfo1_pw=0.2 lfo2=sine:9:0 lfo2_pitch=20 drive=0.6
inst sfx_zap   synth bus=sfx wave=pulse pw=0.3 pitchenv=24 pitchtime=0.06 attack=0 decay=0.04 sustain=0.5 release=0.03 drive=0.8
inst sfx_beam  synth bus=sfx wave=square pitchenv=36 pitchtime=0.11 attack=0 decay=0.08 sustain=0.6 release=0.06 lfo1=sine:40:0 lfo1_pitch=30 drive=0.6
inst sfx_hiss  synth bus=sfx wave=noise filter=lp cutoff=9000 fenv=-3 fattack=0 fdecay=0.08 fsustain=0 attack=0 decay=0.06 sustain=0 release=0.04
inst sfx_whip  synth bus=sfx wave=pulse pw=0.3 pitchenv=-24 pitchtime=0.055 attack=0 decay=0.04 sustain=0.4 release=0.04 drive=0.7
inst sfx_breath synth bus=sfx wave=noise filter=bp cutoff=1500 res=0.3 fenv=2 fattack=0.05 fdecay=0.05 fsustain=1 attack=0 decay=0.05 sustain=0 release=0.04
inst sfx_rise  synth bus=sfx wave=pulse pw=0.2 pitchenv=-36 pitchtime=0.42 attack=0.01 decay=0.2 sustain=0.7 release=0.2 lfo1=sine:6:0.1 lfo1_pitch=15 send=sfx_echo:-4 drive=0.5
inst sfx_air   synth bus=sfx wave=noise filter=bp cutoff=800 res=0.3 fenv=3 fattack=0.42 fdecay=0.2 fsustain=0.5 attack=0.12 decay=0.2 sustain=0.2 release=0.22 send=sfx_echo:-6
inst sfx_rumble synth bus=sfx wave=noise filter=lp cutoff=300 res=0.2 attack=0.22 decay=0.2 sustain=0.3 release=0.26
inst sfx_drone synth bus=sfx wave=triangle attack=0.18 decay=0.2 sustain=0.5 release=0.26 lfo1=sine:5:0 lfo1_pitch=40
inst sfx_blast synth bus=sfx wave=noise filter=lp cutoff=12000 fenv=-3 fattack=0 fdecay=0.26 fsustain=0 attack=0 decay=0.26 sustain=0 release=0.26 send=sfx_room:-6
inst sfx_thump synth bus=sfx wave=sine pitchenv=7 pitchtime=0.14 attack=0 decay=0.18 sustain=0 release=0.12 drive=0.6
inst sfx_fall  synth bus=sfx wave=triangle attack=0 decay=0.06 sustain=0.5 release=0.09 drive=0.5
inst sfx_blip  synth bus=sfx wave=pulse pw=0.25 attack=0 decay=0.03 sustain=0.3 release=0.04 drive=0.6
inst sfx_grind synth bus=sfx wave=saw filter=lp cutoff=1800 res=0.2 glide=0.02 mono=1 attack=0 decay=0.06 sustain=0.5 release=0.08 drive=0.6
inst sfx_tick  synth bus=sfx wave=pulse pw=0.25 attack=0 decay=0.03 sustain=0.1 release=0.04 drive=0.6
