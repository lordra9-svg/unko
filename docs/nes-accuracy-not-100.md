# NES games not at 100% accuracy (ares primary)

Generated from `compatibility/accuracy-ares.jsonl` — **ares** primary, 120 frames. RustyNES remains secondary.

**Metric:** NTSC uses palette-independent steady-state geometry from frame 30; PAL uses the calibrated final-frame exact-pixel comparison (`ARES_REGION=PAL`, 288-line canvas cropped to 256x240).

- NES archives: **7101**
- Measured: **7069**
- Below 100% by the relevant ares metric: **2699**
- No measurement / oracle error: **32**
- Total needing attention: **2731**

| Band | Games |
|---|---:|
| 0–50% | 1727 |
| 50–90% | 308 |
| 90–99% | 644 |
| 99–100% | 20 |

Full list: [`nes-accuracy-not-100.csv`](nes-accuracy-not-100.csv)

## Worst 50 measured games

| Accuracy | Metric | Game |
|---:|---|---|
| 0.00% | steady | '93 Zuixin Yizhi Chaoji Jinka 4-in-1 (China) (Unl) |
| 0.00% | steady | 100-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 100-in-1 Contra Function 16 (Asia) (En) (Pirate) |
| 0.00% | steady | 100-in-1 Real Game (China) (En,Ja) (Pirate) |
| 0.00% | steady | 10000000-in-1 Unchained Melody 95 (Asia) (En) (Pirate) |
| 0.00% | steady | 117-in-1 (Asia) (En) (WQ1708 D) (Pirate) |
| 0.00% | steady | 1200-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 150-in-1 Golden 1992 (Asia) (En) (Pirate) |
| 0.00% | steady | 16-in-1 Cassetta Videogame (Italy) (En) (Pirate) |
| 0.00% | steady | 168-in-1 (Asia) (En,Ja) (ABCARD-15) (Pirate) |
| 0.00% | steady | 168-in-1 New Contra Function 16 (Asia) (En) (Pirate) |
| 0.00% | steady | 18-in-1 - Videogame Super 8 Bit - Cassetta 18 Giochi (Italy) (En) (Aftermarket) (Pirate) |
| 0.00% | steady | 1992 Super HiK 3-in-1 (Asia) (En,Ja) (Pirate) |
| 0.00% | steady | 1995 Super Value Collection 2-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 1997 Super HIK 16-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 2-in-1 - Conan & Rygar (Asia) (En) (Pirate) |
| 0.00% | steady | 2-in-1 - Conan & Rygar (Asia) (En) (RC58) (Pirate) |
| 0.00% | steady | 2-in-1 - Shuang Wang Ji (Asia) (Pirate) |
| 0.00% | steady | 2-in-1 - Street Dance + Hit Mouse (China) (En) (Aftermarket) (Unl) |
| 0.00% | steady | 2002 121-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 2002 61-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 22 Games (China) (En) (Pirate) |
| 0.00% | steady | 22 Games (China) (En) (Tsang Hai) (Pirate) |
| 0.00% | steady | 23+222-in-1 (245-in-1 Real Game) (China) (Aftermarket) (Pirate) |
| 0.00% | steady | 23-in-1 Super Game (Europe) (En,Ja) (Pirate) |
| 0.00% | steady | 23-in-1 Super Game (Europe) (K23-001) (Pirate) |
| 0.00% | steady | 24K 4-in-1 Baozheng Zhen Ka Quanji (Asia) (En) (Pirate) |
| 0.00% | steady | 29~10MG-in-1 (Asia) (En,Ja) (Pirate) |
| 0.00% | steady | 3-in-1 - Spartan Super 1998 (Asia) (En) (Pirate) |
| 0.00% | steady | 30 Games (Brazil) (En) (Pirate) |
| 0.00% | steady | 32-in-1 Speech Educational Software (Asia) (En) (Pirate) |
| 0.00% | steady | 3D Block (Asia) (En) (Hwang Shinwei) (Unl) |
| 0.00% | steady | 3D Block (Taiwan) (En) (Hwang Shinwei) (Pirate) |
| 0.00% | steady | 4-in-1 (AP009) (Asia) (En) (Pirate) |
| 0.00% | steady | 4-in-1 (Asia) (En) (AP009) (Pirate) |
| 0.00% | steady | 4-in-1 (CK-001) (Asia) (En) (Pirate) |
| 0.00% | steady | 4-in-1 (CK-001) (Taiwan) (En) (Pirate) |
| 0.00% | steady | 4-in-1 (SN 406) (Asia) (En) (Pirate) |
| 0.00% | steady | 4-in-1 - BattleCity - Pac-Man - Galaga - Ice Climber (Asia) (En) (Pirate) |
| 0.00% | steady | 400-in-1 (Asia) (En) (NTDEC) (Pirate) |
| 0.00% | steady | 42 Games (China) (En) (Pirate) |
| 0.00% | steady | 42 Games (China) (En) (Tsang Hai) (Pirate) |
| 0.00% | steady | 42-in-1 (Asia) (En) (Pirate) |
| 0.00% | steady | 45-in-1 Educational Computer (Asia) (En) (Pirate) |
| 0.00% | steady | 48-in-1 Educational Computer (Spain) (Pirate) |
| 0.00% | steady | 48-in-1 Educational Computer 2000 (Asia) (En) (Pirate) |
| 0.00% | steady | 5-in-1 (1993) (Asia) (En) (Unl) |
| 0.00% | steady | 500-in-1 - Unchained Melody (Asia) (En) (Pirate) |
| 0.00% | steady | 52-in-1 (Asia) (En) (VT302) (Pirate) |
| 0.00% | steady | 52-in-1 (VT302) (Asia) (En) (Pirate) |

## No measurement / oracle errors

| Game | Error |
|---|---|
| Bingo (Asia) (TXC) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Bingo (Taiwan) (En) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Controller Test Program (Japan) (En) (Test Program) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Fuzhou Coolboy 400-in-1 Real Game (World) (Aftermarket) (Pirate) | ours: unko: ROM is larger than the 67108864-byte limit |
| Galaxian (Japan) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (En) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (En) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (Japan) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (World) (Namcot Collection, Namco Museum Archives Vol 1) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Galaxian (World) (Rev 1) (Evercade) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Kart Racer '86 (World) (Proto) (2022-10-26) (879cbea) (Aftermarket) (Unl) | ares: ares: frame dump missing (<tmp> |
| Magi Cube (USA) (Proto) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Magic Giral (Meinu Quan) (Taiwan) (Unl) | ours: unko: invalid ROM: image truncated: expected 32784 bytes, got 8208 |
| Magic Jewelry (Asia) (En) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Magic Jewelry (Taiwan) (En) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Super Mario Bros. 2 (Asia) (En) (09-034A PCB) (LE10) (Pirate) | ours: unko: invalid ROM: image truncated: expected 172048 bytes, got 49168 |
| Super Mario Bros. 2 (Asia) (En) (09-034A PCB) (Pirate) | ours: unko: invalid ROM: image truncated: expected 172048 bytes, got 49168 |
| Test Ver. 1.01 - Dlya Proverki TV Pristavok (Russia) (RT-01) (Unl) | ours: unko: invalid ROM: image truncated: expected 81936 bytes, got 18448 |
| Thexder (Japan) (En) | ares: ares: frame dump missing (<tmp> |
| Volleyball (Asia) (En) (LE08) (Pirate) | ours: unko: invalid ROM: image truncated: expected 172048 bytes, got 49168 |
| Volleyball (Asia) (En) (Pirate) | ours: unko: invalid ROM: image truncated: expected 172048 bytes, got 49168 |
| Xiao Mali (Asia) (C&E) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Xiao Mali (Asia) (Idea-Tek) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Xiao Mali (Asia) (Nei-Hu) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Xiao Mali (Taiwan) (En) (C&E) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Xiao Mali (Taiwan) (En) (Idea-Tek) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Xiao Mali (Taiwan) (En) (Nei-Hu) (Unl) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| Yoshi's Cookie (USA) (Virtual Console) | ares: ares: frame dump missing (<tmp> |
| [BIOS] Demo Vision (USA) (Rev 2) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| [BIOS] Sharp My Computer TV C1 (Japan) | ours: unko: invalid ROM: image truncated: expected 40976 bytes, got 16400 |
| [BIOS] Wide Boy (World) (Rev 1) | ours: unko: invalid ROM: image truncated: expected 32784 bytes, got 8208 |
| [BIOS] Wide Boy-3 (Japan) | ours: unko: invalid ROM: image truncated: expected 32784 bytes, got 8208 |

