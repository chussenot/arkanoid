# The machine: analog-style synthesis (studio_synth). Bass and pulse low,
# sparkle high; the middle belongs to the orchestra and the game.

# A dark, punchy saw bass: filter nearly closed, opened by a short envelope.
inst synth_bass synth bus=electronic gain=-4 pan=0 send=room:-18 wave=saw stack=2 detune=6 spread=0.15 sub=0.55 filter=ladder cutoff=110 res=0.35 keytrack=0.4 fenv=2.6 fattack=0.001 fdecay=0.16 fsustain=0.12 frelease=0.1 velcutoff=1.0 drive=0.25 attack=0.002 decay=0.3 sustain=0.75 release=0.07
# The sequenced pulse: octave-leaping sixteenths, tight and filtered.
inst synth_sequence synth bus=electronic gain=-9 pan=0 send=echo:-10 send=stage:-14 wave=pulse pw=0.32 stack=2 detune=5 spread=0.5 filter=lp cutoff=700 res=0.45 keytrack=0.6 fenv=2.2 fattack=0.001 fdecay=0.11 fsustain=0.0 frelease=0.05 attack=0.001 decay=0.18 sustain=0.2 release=0.04 lfo2=tri:0.07:0 lfo2_cutoff=0.6
# A wide, slowly moving pad under the orchestra.
inst synth_pad synth bus=electronic gain=-12 send=hall:-6 send=long:-12 wave=saw stack=5 detune=14 spread=0.85 filter=lp cutoff=1400 res=0.12 keytrack=0.3 attack=1.4 decay=1.0 sustain=0.85 release=2.6 lfo1=tri:0.11:0 lfo1_cutoff=0.5 lfo2=sine:0.23:0.5 lfo2_pan=0.15
# A lead to double or answer a theme: saw, gentle delayed vibrato, glide.
inst synth_lead synth bus=electronic gain=-9 send=echo:-12 send=hall:-10 wave=saw stack=3 detune=7 spread=0.35 filter=lp cutoff=2600 res=0.2 keytrack=0.5 attack=0.02 decay=0.4 sustain=0.8 release=0.35 glide=0.05 mono=1 lfo1=sine:5.2:0.35 lfo1_pitch=14
# Glass: Spark's luminous bell-like arpeggio, high and short with long echoes.
inst synth_glass synth bus=electronic gain=-10 pan=0.1 send=echo:-6 send=hall:-8 wave=triangle stack=2 detune=4 spread=0.6 filter=hp cutoff=500 attack=0.001 decay=0.9 sustain=0.0 release=0.9
# The drone: a low bed that breathes.
inst electronic_drone synth bus=electronic gain=-13 send=long:-8 wave=saw stack=4 detune=6 spread=0.7 sub=0.4 noise=0.04 filter=ladder cutoff=260 res=0.3 keytrack=0.2 attack=3.0 decay=1.0 sustain=0.9 release=4.0 lfo1=sine:0.06:0 lfo1_cutoff=0.9
# Air: filtered noise for atmospheres and transitions.
inst electronic_air synth bus=electronic gain=-20 send=long:-6 wave=noise filter=bp cutoff=2400 res=0.35 attack=1.5 decay=0.5 sustain=0.8 release=2.5 lfo1=sine:0.09:0 lfo1_cutoff=1.2
# The electronic pulse: a sine thump that keys the pad's breathing.
inst electronic_pulse synth bus=electronic gain=-8 wave=sine pitchenv=18 pitchtime=0.045 attack=0.001 decay=0.22 sustain=0.0 release=0.05 drive=0.2
# Metal: short bright ticks.
inst electronic_metal synth bus=percussion gain=-16 pan=0.25 send=room:-10 wave=noise filter=hp cutoff=7000 res=0.2 attack=0.0005 decay=0.045 sustain=0.0 release=0.03
