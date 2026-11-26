# NES games not at 100% accuracy

Generated from `compatibility/accuracy.jsonl` (RustyNES NES oracle, 120 frames).

**Metric note:** full-run frame-0 geometry is never exactly 100% because the oracle and Unko differ during boot. This list therefore uses the **steady-state geometry from frame 30 onward**. If “accuracy” means the displayed full-run R percentage, all measured games are below 100%.

- NES archives: **7101**
- Measured: **6726**
- Measured but steady-state below 100%: **1835**
- No measurement / oracle error: **375**
- Total needing attention: **2210**

| Steady-state band | Games |
|---|---:|
| 0–50% | 892 |
| 50–90% | 279 |
| 90–99% | 664 |
| 99–100% | 0 |

## Known oracle disagreement

`Argus (Japan)` and `Argus (Japan) (En)` are excluded from this list: RustyNES renders a spurious tile-0 overlay, while ares matches Unko at **100% steady-state geometry** from frame 30. See `compatibility/accuracy-ares.jsonl`.

Full list: [`nes-accuracy-not-100.csv`](nes-accuracy-not-100.csv)

## Worst 50 measured games

| Steady | Best full-run | Game |
|---:|---:|---|
| 0.00% | 0.00% | Adventures of Lex and Grim, The (World) (Aftermarket) (Unl) |
| 0.00% | 0.00% | Super Mario & Sonic 2 (Asia) (Ja) (Pirate) |
| 0.00% | 0.00% | Super Mario & Sonic 2 (Asia) (Ja) (v1.0) (Pirate) |
| 0.00% | 0.83% | SoundTest 2 (World) (Program) (Aftermarket) (Unl) |
| 0.00% | 0.83% | cajoNES (World) (Music Program) (Aftermarket) (Unl) |
| 0.00% | 0.83% | cajoNES DELUXE! (World) (Music Program) (Aftermarket) (Unl) |
| 0.00% | 0.85% | World Hero (Taiwan) (En) (Pirate) |
| 0.00% | 0.85% | World Hero (Taiwan) (En) (Pirate) |
| 0.00% | 0.87% | Karate Kick (World) (NESDev 2016) (Aftermarket) (Unl) |
| 0.00% | 0.87% | Legends of Owlia, The (World) (Aftermarket) (Unl) |
| 0.00% | 0.87% | SMMM (World) (v1.01) (Music Program) (Aftermarket) (Unl) |
| 0.00% | 1.67% | Chinese Checkers (Taiwan) (En) (Unl) |
| 0.00% | 1.67% | Chinese Checkers (Taiwan) (En) (Unl) |
| 0.00% | 1.67% | Happy Pairs (Taiwan) (En) (Unl) |
| 0.00% | 1.67% | Happy Pairs (Taiwan) (En) (Unl) |
| 0.00% | 1.67% | Mission Cobra (USA) (Unl) |
| 0.00% | 1.67% | Puzzle (Taiwan) (En) (Idea-Tek) (Unl) |
| 0.00% | 1.67% | Puzzle (Taiwan) (En) (Unl) |
| 0.00% | 1.67% | Rise of Amondus, The (World) (Demo) (Aftermarket) (Unl) |
| 0.00% | 1.67% | Super Pang (Asia) (En) (Unl) |
| 0.00% | 1.67% | Twin Eagle (Asia) (PAL) (Unl) |
| 0.00% | 1.67% | Twin Eagle (Asia) (PAL) (Unl) |
| 0.00% | 1.67% | World Hero (Taiwan) (En) (v1.1) (Pirate) |
| 0.00% | 1.67% | World Hero (Taiwan) (En) (v1.1) (Pirate) |
| 0.00% | 1.68% | Mad Wizard, The - A Candelabra Chronicle (World) (Demo) (2015-06-15) (Aftermarket) (Unl) |
| 0.00% | 1.68% | Ninja Jajamaru-kun (Japan) (Retro-Bit Generations) |
| 0.00% | 1.74% | Fighting Hero (Taiwan) (En) (Unl) |
| 0.00% | 1.74% | Fighting Hero (Taiwan) (En) (Unl) |
| 0.00% | 1.74% | Hassle (Asia) (En) (Pirate) |
| 0.00% | 1.74% | IronSword - Wizards & Warriors II (Europe) |
| 0.00% | 1.74% | IronSword - Wizards & Warriors II (USA) |
| 0.00% | 1.74% | Oulu Zhi Zhan (China) (Pirate) |
| 0.00% | 1.74% | Seicross (Japan) (En) |
| 0.00% | 1.74% | Seicross (Japan) (Rev 1) |
| 0.00% | 1.74% | Street Fighter III (Asia) (En) (18 Fighters) (Pirate) |
| 0.00% | 1.74% | Street Fighter III (Asia) (En) (18 Fighters) (Pirate) |
| 0.00% | 1.74% | Street Fighter III (Asia) (En) (9 Fighters) (Pirate) |
| 0.00% | 1.74% | Street Fighter III (Asia) (En) (9 Fighters) (Pirate) |
| 0.00% | 1.74% | Zhengba Shiji (China) (960268) (Pirate) |
| 0.00% | 1.74% | Zhengba Shiji (China) (Pirate) |
| 0.00% | 2.50% | 999-in-1 (Asia) (En) (Pirate) |
| 0.00% | 2.50% | 999-in-1 (Asia) (En) (Pirate) |
| 0.00% | 2.50% | God Slayer - Haruka Tenkuu no Sonata (Japan) |
| 0.00% | 2.50% | God Slayer - Haruka Tenkuu no Sonata (Japan) (SNK 40th Anniversary Collection) |
| 0.00% | 2.50% | Indiana Jones and the Temple of Doom (USA) (Beta 6) |
| 0.00% | 2.50% | Indiana Jones and the Temple of Doom (USA) (V1.1) (Unl) |
| 0.00% | 2.50% | Indiana Jones and the Temple of Doom (USA) (V1.1) (Unl) |
| 0.00% | 2.50% | Joshua & the Battle of Jericho (USA) (v6.0) (Unl) |
| 0.00% | 2.50% | Sea of Dreamland (Europe, Asia) (En) (Unl) |
| 0.00% | 2.50% | Solar Wars (World) (v1.0) (Aftermarket) (Unl) |

## No measurement / oracle errors

| Game | Error |
|---|---|
| 100-in-1 (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(576) |
| 1200-in-1 (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(380) |
| 16-in-1 Cassetta Videogame (Italy) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(422) |
| 1995 Super Value Collection 2-in-1 (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(396) |
| 1996 Yingyu CAI 3-in-1 (China) (Unl) | rustynes: load <tmp>: UnsupportedMapper(519) |
| 2-in-1 - Conan & Rygar (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(396) |
| 2-in-1 - Conan & Rygar (Asia) (En) (RC58) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(396) |
| 2-in-1 - Street Dance + Hit Mouse (China) (En) (Aftermarket) (Unl) | rustynes: load <tmp>: UnsupportedMapper(256) |
| 2002 121-in-1 (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(332) |
| 2002 61-in-1 (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(332) |
| 29~10MG-in-1 (Asia) (En,Ja) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(351) |
| 3-in-1 - Spartan Super 1998 (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(519) |
| 30 Games (Brazil) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(314) |
| 3D Block (Taiwan) (En) (Hwang Shinwei) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(355) |
| 3D Block (Taiwan) (En) (Hwang Shinwei) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(355) |
| 4-in-1 (Asia) (En) (AP009) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(313) |
| 4-in-1 (Asia) (En) (AP009) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(313) |
| 4-in-1 - Unsurpass Air-Combat Series (Asia) (Ja) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(374) |
| 4-in-1 - Unsurpass Air-Combat Series (Asia) (Ja) (TY018) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(374) |
| 5-in-1 (1993) (Asia) (En) (Unl) | rustynes: load <tmp>: UnsupportedMapper(334) |
| 6-in-1 (Asia) (En) (VIP002) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(394) |
| 62-in-1 Hi Game 2000 (Taiwan) (Ja) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(59) |
| 62-in-1 Hi Game 2000 (Taiwan) (Ja) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(59) |
| 777 Lucky Bingo (Asia) (En) (Unl) | rustynes: load <tmp>: InvalidConfig("mapper save state invalid: mapper 79 PRG-ROM size 16384 is not a non-zero multiple of 32 KiB") |
| 777 Xingyun Lunpan (Taiwan) (En) (Unl) | rustynes: load <tmp>: InvalidConfig("mapper save state invalid: mapper 79 PRG-ROM size 16384 is not a non-zero multiple of 32 KiB") |
| 777 Xingyun Lunpan (Taiwan) (En) (Unl) | rustynes: load <tmp>: InvalidConfig("mapper save state invalid: mapper 79 PRG-ROM size 16384 is not a non-zero multiple of 32 KiB") |
| 777 Xingyun Lunpan (Taiwan) (v1.1) (Unl) | rustynes: load <tmp>: InvalidConfig("mapper save state invalid: mapper 79 PRG-ROM size 16384 is not a non-zero multiple of 32 KiB") |
| 8-BIT XMAS 2017 (World) (Aftermarket) (Unl) | rustynes: load <tmp>: UnsupportedMapper(400) |
| 9999999-in-1 - 14 Giochi X-140 (Italy) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(319) |
| 9999999-in-1 - World Series Championship (Asia) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(59) |
| 9999999-in-1 - World Series Championship (Taiwan) (En) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(59) |
| 9999999-in-1 - World Series Championship (Taiwan) (En) (v1.1) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(59) |
| Ai Senshi Nicol (Asia) (Ja) (Whirlwind Manu) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(309) |
| Ai Senshi Nicol (Asia) (Ja) (Whirlwind Manu) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(309) |
| Akumajou Dracula (Asia) (En) (Kaiser) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(554) |
| Akumajou Dracula (Asia) (Ja) (Kaiser) (KS-7010) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(554) |
| Alter Ego (World) (Aftermarket) (Unl) | rustynes: load <tmp>: InvalidConfig("mapper save state invalid: NROM expects 16 KiB or 32 KiB PRG-ROM, got 49152 bytes") |
| Anamanaguchi - Power Supply (USA) (Aftermarket) (Unl) | rustynes: load <tmp>: UnsupportedMapper(399) |
| Aoshi Tiandi (China) (Aftermarket) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(224) |
| Armana no Kiseki (Asia) (Ja) (Co Tung) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(548) |
| Armana no Kiseki (Asia) (Ja) (Co Tung) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(548) |
| Atari Flashback (USA) (Aftermarket) (Unl) | rustynes: load <tmp>: UnsupportedMapper(534) |
| Batman - The Video Game (Asia) (En) (Fine Studio) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(417) |
| Batman - The Video Game (Asia) (En) (NTDEC) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(308) |
| Battletoads-Double Dragon (USA, Europe) (Retro-Bit) | rustynes: load <tmp>: UnsupportedMapper(470) |
| Bawang de Dalu - Zhongwen Jiaqiang Ban (China) (Pirate) | rustynes: load <tmp>: UnsupportedMapper(544) |
| Bingo (Taiwan) (En) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Bingo (Taiwan) (En) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Block Force (Taiwan) (En) (Unl) | rustynes: load <tmp>: UnsupportedMapper(355) |
| Block Force (Taiwan) (En) (Unl) | rustynes: load <tmp>: UnsupportedMapper(355) |
| … | 325 more in the CSV |

