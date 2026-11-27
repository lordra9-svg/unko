# NES games not at 100% accuracy (ares primary)

Generated from `compatibility/accuracy-ares.jsonl` - **ares** primary, 120 frames. RustyNES remains secondary.

**Metric:** newly re-run letters use the NES palette-index agreement over frames 30+ (`video_pct`); records not yet revisited keep their historical steady-geometry or final-frame value.

- NES archives: **7101**
- Measured: **7051**
- Measured with the palette-index metric: **1308**
- Below 100% by the relevant ares metric: **527**
- Ignored tagged dumps below 100%: **1854**
- No measurement / oracle error: **27**
- Ignored tagged dumps with errors: **23**
- Total needing attention: **554**

| Band | Games |
|---|---:|
| 0-50% | 30 |
| 50-90% | 47 |
| 90-99% | 102 |
| 99-100% | 348 |

Full list: [`nes-accuracy-not-100.csv`](nes-accuracy-not-100.csv)

## Worst 50 measured games

| Accuracy | Metric | Game |
|---:|---|---|
| 0.00% | index | Athena (Japan) |
| 0.00% | index | Athena (Japan) (En) |
| 0.00% | index | Athena (USA) |
| 0.00% | index | Bionic Commando (USA) (Capcom Classics Mini Mix) |
| 0.00% | index | Bionic Commando (USA) (Capcom Classics Mini Mix) [b] |
| 0.00% | index | Esper Dream 2 - Aratanaru Tatakai (Japan) |
| 0.00% | index | Fleet Commander (Japan) |
| 0.00% | index | Ganbare Goemon Gaiden - Kieta Ougon Kiseru (Japan) (Rev 1) (Virtual Console) [b] |
| 0.00% | index | Gegege no Kitarou 2 - Youkai Gundan no Chousen (Japan) |
| 0.00% | index | Kid Dracula (World) (Castlevania Anniversary Collection) |
| 0.00% | index | Mighty Final Fight (USA) (Capcom Classics Mini Mix) |
| 0.00% | index | Minna no Taabou no Nakayoshi Daisakusen (Japan) |
| 0.00% | index | Mouryou Senki Madara (Japan) |
| 0.00% | index | Nantettatte!! Baseball + Nantettatte!! Baseball - Ko-Game Cassette - '91 Kaimaku Hen (Japan) |
| 0.00% | index | Nantettatte!! Baseball + Nantettatte!! Baseball - Ko-Game Cassette - OB All Star Hen (Japan) |
| 0.00% | index | Pocket Zaurus - Juu Ouken no Nazo (Japan) |
| 0.00% | index | Rod Land (World) (Retro-Bit) (256KiB) |
| 0.00% | index | Rod Land (World) (Retro-Bit) (512KiB) |
| 0.00% | index | Star Wars (USA) (Limited Run Games) |
| 0.00% | index | Strider (USA) (Capcom Classics Mini Mix) |
| 10.00% | index | Kaguya Hime Densetsu (Japan) |
| 26.63% | index | Racer Mini Yonku - Japan Cup (Japan) |
| 27.69% | index | Devil Man (Japan) |
| 36.67% | index | Lagrange Point (Japan) |
| 37.78% | index | Future Wars - Mirai Senshi Lios (Japan) |
| 38.89% | index | Suikoden - Tenmei no Chikai (Japan) |
| 41.49% | index | Super Pitfall (USA) |
| 41.55% | index | Super Pitfall (Japan) |
| 45.36% | index | Digital Devil Story - Megami Tensei II (Japan) |
| 45.36% | index | Digital Devil Story - Megami Tensei II (Japan) (Rev 1) |
| 50.00% | index | Murder Club - Honkaku Mystery Adventure (Japan) |
| 52.87% | index | Ninjara Hoi! (Japan) |
| 56.67% | index | Twin Cobra (USA) |
| 56.67% | index | Kyuukyoku Tiger (Japan) |
| 58.89% | steady | 100 Man Dollar Kid - Maboroshi no Teiou Hen (Japan) |
| 58.89% | index | Hydlide 3 - Yami kara no Houmonsha (Japan) |
| 60.00% | index | Otaku no Seiza - An Adventure in the Otaku Galaxy (Japan) |
| 61.11% | index | Swords and Serpents (Europe) |
| 61.11% | index | Swords and Serpents (USA) |
| 62.34% | index | Oeka Kids - Anpanman no Hiragana Daisuki (Japan) |
| 62.92% | index | Swords and Serpents (France) |
| 65.56% | index | Major League Baseball (USA) |
| 65.56% | index | Major League Baseball (USA) (Rev 1) |
| 68.77% | index | Bikkuriman World - Gekitou Sei Senshi (Japan) |
| 70.00% | index | Family Quiz - 4-nin wa Rival (Japan) |
| 71.26% | steady | 8 Eyes (USA, Europe) (Pixel Heart) |
| 71.88% | index | Arkanoid II (Japan) (En) |
| 72.22% | index | Total Recall (USA) |
| 73.10% | index | Treasure Master (USA) |
| 75.51% | index | Terminator 2 - Judgment Day (Europe) |

## No measurement / oracle errors

| Game | Error |
|---|---|
| Battletoads-Double Dragon (USA, Europe) (Retro-Bit) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Controller Test Program (Japan) (En) (Test Program) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| David Crane's A Boy and His Blob - Trouble on Blobolonia (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Galaxian (Japan) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (En) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (En) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (World) (Namcot Collection, Namco Museum Archives Vol 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (World) (Rev 1) (Evercade) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Idemitsu - Space College - Kikenbutsu no Yasashii Butsuri to Kagaku (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Kyuukyoku Harikiri Koushien (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Kyuukyoku Harikiri Stadium - Heisei Gannen Ban (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Kyuukyoku Harikiri Stadium III (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Maniac Mansion (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| NHK Gakuen - Space School - Sansu 5 Nen (Jou) (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| NHK Gakuen - Space School - Sansu 6 Nen (Ge) (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| NHK Gakuen - Space School - Sansu 6 Nen (Jou) (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Nintendo Campus Challenge 1991 (USA) (Competition Cart) [b] | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Nintendo World Championships 1990 (USA) (Competition Cart) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| S.C.A.T. - Special Cybernetic Attack Team (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| SD Keiji - Blader (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Shadow of the Ninja (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Target - Renegade (USA) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Target - Renegade (World) (Evercade) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Thunderbolt Fighting Plane (World) (QUByte Classics) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Total Recall (Europe) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Ultima - Warriors of Destiny (USA) | ares capture is a uniform blank frame (board unsupported by the oracle) |

