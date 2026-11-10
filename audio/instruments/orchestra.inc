# The orchestra: VSCO 2 CE samples (audio/samples/, CC0). Seating, from the
# audience: violins left, violas centre, celli and basses right; horns left
# of centre, trumpets centre, trombones and tuba right. Sections built from
# solo players are stacked (copies detuned and spread).

# Strings
inst strings_high      sfz=../samples/violins_sus.sfz  bus=orchestra pan=-0.45 gain=-2 send=hall:-8  attack=0.14 release=0.55
inst strings_mid       sfz=../samples/violas_sus.sfz   bus=orchestra pan=-0.05 gain=-3 send=hall:-8  attack=0.14 release=0.55
inst strings_low       sfz=../samples/celli_sus.sfz    bus=orchestra pan=0.35  gain=-2 send=hall:-9  attack=0.12 release=0.5
inst strings_bass      sfz=../samples/basses_sus.sfz   bus=orchestra pan=0.55  gain=-3 send=hall:-10 attack=0.1  release=0.45 stack=2 stack_detune=7 stack_delay=0.018 spread=0.25
inst strings_high_spic sfz=../samples/violins_spic.sfz bus=orchestra pan=-0.45 gain=-3 send=hall:-10 release=0.12
inst strings_mid_spic  sfz=../samples/violas_spic.sfz  bus=orchestra pan=-0.05 gain=-4 send=hall:-10 release=0.12
inst strings_low_spic  sfz=../samples/celli_spic.sfz   bus=orchestra pan=0.35  gain=-3 send=hall:-11 release=0.12
inst strings_bass_spic sfz=../samples/basses_spic.sfz  bus=orchestra pan=0.55  gain=-4 send=hall:-12 release=0.12 stack=2 stack_detune=6 stack_delay=0.01 spread=0.2
inst strings_high_trem sfz=../samples/violins_trem.sfz bus=orchestra pan=-0.45 gain=-4 send=hall:-8  attack=0.2 release=0.6
inst strings_mid_trem  sfz=../samples/violas_trem.sfz  bus=orchestra pan=-0.05 gain=-5 send=hall:-8  attack=0.2 release=0.6
inst strings_low_trem  sfz=../samples/celli_trem.sfz   bus=orchestra pan=0.35  gain=-4 send=hall:-9  attack=0.2 release=0.6
inst strings_high_pizz sfz=../samples/violins_pizz.sfz bus=orchestra pan=-0.45 gain=-4 send=hall:-9  release=0.3
inst strings_low_pizz  sfz=../samples/celli_pizz.sfz   bus=orchestra pan=0.35  gain=-3 send=hall:-10 release=0.35

# Brass
inst brass_horns        sfz=../samples/horn_sus.sfz     bus=orchestra pan=-0.2 gain=-4 send=hall:-6 attack=0.09 release=0.6 stack=3 stack_detune=9 stack_delay=0.02 spread=0.45
inst brass_horns_stac   sfz=../samples/horn_stac.sfz    bus=orchestra pan=-0.2 gain=-5 send=hall:-7 release=0.15 stack=3 stack_detune=8 stack_delay=0.008 spread=0.45
inst brass_trumpets     sfz=../samples/trumpet_sus.sfz  bus=orchestra pan=0.05 gain=-6 send=hall:-7 attack=0.06 release=0.5 stack=2 stack_detune=6 stack_delay=0.015 spread=0.3
inst brass_trumpets_stac sfz=../samples/trumpet_stac.sfz bus=orchestra pan=0.05 gain=-7 send=hall:-8 release=0.12 stack=2 stack_detune=6 stack_delay=0.006 spread=0.3
inst brass_trombones    sfz=../samples/trombone_sus.sfz bus=orchestra pan=0.3  gain=-5 send=hall:-7 attack=0.07 release=0.5 stack=2 stack_detune=7 stack_delay=0.016 spread=0.3
inst brass_trombones_stac sfz=../samples/trombone_stac.sfz bus=orchestra pan=0.3 gain=-6 send=hall:-8 release=0.12 stack=2 stack_detune=7 stack_delay=0.006 spread=0.3
inst brass_low          sfz=../samples/tuba_sus.sfz     bus=orchestra pan=0.4  gain=-4 send=hall:-9 attack=0.08 release=0.5
inst brass_low_stac     sfz=../samples/tuba_stac.sfz    bus=orchestra pan=0.4  gain=-5 send=hall:-10 release=0.15

# Percussion
inst timpani      sfz=../samples/timpani.sfz      bus=percussion pan=0.15 gain=-2 send=hall:-6 release=1.5
inst timpani_roll sfz=../samples/timpani_roll.sfz bus=percussion pan=0.15 gain=-4 send=hall:-6 release=0.8
inst percussion   sfz=../samples/kit.sfz          bus=percussion pan=0.0  gain=-4 send=hall:-5 release=2.0
