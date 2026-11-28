# NES games not at 100% accuracy (ares primary)

Generated from `compatibility/accuracy-ares.jsonl` - **ares** primary, 120 frames. RustyNES remains secondary.

**Metric:** newly re-run letters use the NES palette-index agreement over frames 30+ (`video_pct`); records not yet revisited keep their historical steady-geometry or final-frame value.

- NES archives: **7101**
- Measured: **7047**
- Measured with the palette-index metric: **1316**
- Below 100% by the relevant ares metric: **513**
- Ignored tagged dumps below 100%: **1847**
- No measurement / oracle error: **37**
- Ignored tagged dumps with errors: **26**
- Total needing attention: **550**

| Band | Games |
|---|---:|
| 0-50% | 15 |
| 50-90% | 15 |
| 90-99% | 125 |
| 99-100% | 358 |

Full list: [`nes-accuracy-not-100.csv`](nes-accuracy-not-100.csv)

## Worst 50 measured games

| Accuracy | Metric | Game |
|---:|---|---|
| 0.00% | index | Kid Dracula (World) (Castlevania Anniversary Collection) |
| 0.00% | index | Minna no Taabou no Nakayoshi Daisakusen (Japan) |
| 0.00% | index | Nantettatte!! Baseball + Nantettatte!! Baseball - Ko-Game Cassette - '91 Kaimaku Hen (Japan) |
| 0.00% | index | Nantettatte!! Baseball + Nantettatte!! Baseball - Ko-Game Cassette - OB All Star Hen (Japan) |
| 0.00% | index | Rod Land (World) (Retro-Bit) (256KiB) |
| 0.00% | index | Rod Land (World) (Retro-Bit) (512KiB) |
| 0.00% | index | Star Wars (USA) (Limited Run Games) |
| 26.63% | index | Racer Mini Yonku - Japan Cup (Japan) |
| 27.69% | index | Devil Man (Japan) |
| 38.33% | index | Tokoro-san no Mamoru mo Semeru mo (Japan) |
| 38.89% | index | Suikoden - Tenmei no Chikai (Japan) |
| 41.49% | index | Super Pitfall (USA) |
| 41.55% | index | Super Pitfall (Japan) |
| 45.36% | index | Digital Devil Story - Megami Tensei II (Japan) |
| 45.36% | index | Digital Devil Story - Megami Tensei II (Japan) (Rev 1) |
| 61.11% | index | Swords and Serpents (Europe) |
| 62.92% | index | Swords and Serpents (France) |
| 69.57% | index | Bikkuriman World - Gekitou Sei Senshi (Japan) |
| 73.10% | index | Treasure Master (USA) |
| 75.51% | index | Terminator 2 - Judgment Day (Europe) |
| 75.88% | index | Terminator 2 (Japan) |
| 75.91% | index | Terminator 2 - Judgment Day (USA) |
| 77.45% | index | Nintendo NTF2 System Cartridge (USA) (v1.1) (Test Program) |
| 79.63% | index | Castelian (Europe) |
| 84.08% | index | Super Turrican (Europe) |
| 86.30% | index | Oeka Kids - Anpanman to Oekaki Shiyou!! (Japan) |
| 87.22% | index | Mickey Mouse - Fushigi no Kuni no Daibouken (Japan) (En) |
| 87.54% | index | Mickey Mousecapade (USA) |
| 87.67% | index | Oeka Kids - Anpanman no Hiragana Daisuki (Japan) |
| 89.89% | index | Legend of Prince Valiant, The (Europe) |
| 90.29% | index | Battle Stadium - Senbatsu Pro Yakyuu (Japan) |
| 90.30% | index | Hebereke (Japan) |
| 90.30% | index | Hebereke (Japan) (Virtual Console) |
| 90.42% | index | Flintstones, The - The Rescue of Dino & Hoppy (Japan) |
| 90.67% | index | Ice Hockey (Europe) (Virtual Console) |
| 92.03% | index | Legend of Zelda, The (Europe) |
| 92.03% | index | Legend of Zelda, The (Europe) (Rev 1) |
| 92.03% | index | Legend of Zelda, The (Europe) (Rev 1) (Virtual Console) |
| 92.59% | index | Super Mario Bros. + Duck Hunt (Europe, Asia) (En) |
| 92.73% | index | Super Mario Bros. + Duck Hunt (USA) |
| 93.11% | index | Elite (Europe) (En,Fr,De) |
| 93.17% | index | Dynamite Bowl (Japan) (En) (Rev 1) |
| 93.17% | index | Dynamite Bowl (Japan) (Rev 1) |
| 93.65% | index | Thexder (Japan) |
| 93.65% | index | Thexder (Japan) (En) |
| 93.96% | index | Iron Tank - The Invasion of Normandy (USA) |
| 94.15% | index | Nintendo NTF2 Test Cartridge (USA) (Test Program) |
| 94.23% | index | Ghostbusters II (Europe) |
| 94.51% | index | Dynamite Bowl (Japan) |
| 94.51% | index | Dynamite Bowl (Japan) (En) |

## No measurement / oracle errors

| Game | Error |
|---|---|
| 8 Eyes (USA, Europe) (Pixel Heart) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Athena (Japan) | no scored frames (all frames excluded) |
| Athena (Japan) (En) | no scored frames (all frames excluded) |
| Athena (USA) | no scored frames (all frames excluded) |
| Battletoads-Double Dragon (USA, Europe) (Retro-Bit) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Bionic Commando (USA) (Capcom Classics Mini Mix) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Bionic Commando (USA) (Capcom Classics Mini Mix) [b] | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Controller Test Program (Japan) (En) (Test Program) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| David Crane's A Boy and His Blob - Trouble on Blobolonia (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Fleet Commander (Japan) | no scored frames (all frames excluded) |
| Galaxian (Japan) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (En) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (En) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (World) (Namcot Collection, Namco Museum Archives Vol 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (World) (Rev 1) (Evercade) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Ganbare Goemon Gaiden - Kieta Ougon Kiseru (Japan) (Rev 1) (Virtual Console) [b] | no scored frames (all frames excluded) |
| Idemitsu - Space College - Kikenbutsu no Yasashii Butsuri to Kagaku (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Kyuukyoku Harikiri Koushien (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Kyuukyoku Harikiri Stadium - Heisei Gannen Ban (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Kyuukyoku Harikiri Stadium III (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Maniac Mansion (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Mighty Final Fight (USA) (Capcom Classics Mini Mix) | no scored frames (all frames excluded) |
| NHK Gakuen - Space School - Sansu 5 Nen (Jou) (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| NHK Gakuen - Space School - Sansu 6 Nen (Ge) (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| NHK Gakuen - Space School - Sansu 6 Nen (Jou) (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Nintendo Campus Challenge 1991 (USA) (Competition Cart) [b] | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Nintendo World Championships 1990 (USA) (Competition Cart) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| S.C.A.T. - Special Cybernetic Attack Team (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| SD Keiji - Blader (Japan) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Shadow of the Ninja (USA) (Limited Run Games) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Strider (USA) (Capcom Classics Mini Mix) | no scored frames (all frames excluded) |
| Target - Renegade (USA) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Target - Renegade (World) (Evercade) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Thunderbolt Fighting Plane (World) (QUByte Classics) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Total Recall (Europe) | ares capture is a uniform blank frame (board unsupported by the oracle) |
| Ultima - Warriors of Destiny (USA) | ares capture is a uniform blank frame (board unsupported by the oracle) |

