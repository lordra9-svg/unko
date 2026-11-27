# NES games not at 100% accuracy (ares primary)

Generated from `compatibility/accuracy-ares.jsonl` — the **ares** NES oracle, 120 frames. RustyNES remains a secondary historical cross-check.

**Metric note:** full-run frame-0 geometry is not the stable metric because ares and Unko differ during boot. This list uses **steady-state geometry from frame 30 onward**.

- NES archives: **7101**
- Measured: **7069**
- Measured but steady-state below 100%: **2752**
- No measurement / oracle error: **32**
- Total needing attention: **2784**

| Steady-state band | Games |
|---|---:|
| 0–50% | 2096 |
| 50–90% | 179 |
| 90–99% | 477 |
| 99–100% | 0 |

Full list: [`nes-accuracy-not-100.csv`](nes-accuracy-not-100.csv)

## Worst 50 measured games

| Steady | Best full-run | Game |
|---:|---:|---|
| 0.00% | 0.00% | 117-in-1 (Asia) (En) (WQ1708 D) (Pirate) |
| 0.00% | 0.00% | 3-in-1 - Spartan Super 1998 (Asia) (En) (Pirate) |
| 0.00% | 0.00% | 77-in-1 (Asia) (En) (WQ1806 E) (Pirate) |
| 0.00% | 0.00% | Action in New York (Europe) |
| 0.00% | 0.00% | Addams Family, The (Europe) (En,Fr,De) |
| 0.00% | 0.00% | Addams Family, The - Pugsley's Scavenger Hunt (Europe) |
| 0.00% | 0.00% | Addams Family, The - Pugsley's Scavenger Hunt (Europe) (Beta 1) |
| 0.00% | 0.00% | Addams Family, The - Pugsley's Scavenger Hunt (Europe) (Beta 2) |
| 0.00% | 0.00% | Addams Family, The - Pugsley's Scavenger Hunt (Europe) (Beta 3) |
| 0.00% | 0.00% | Addams Family, The - Pugsley's Scavenger Hunt (Europe) (Beta 4) |
| 0.00% | 0.00% | Addams Family, The - Pugsley's Scavenger Hunt (Europe) (Beta) |
| 0.00% | 0.00% | Adventure Island Classic (Europe) |
| 0.00% | 0.00% | Adventure Island III (Europe) (Proto) |
| 0.00% | 0.00% | Adventure Island Part II, The (Europe) |
| 0.00% | 0.00% | Adventures in the Magic Kingdom (Europe) |
| 0.00% | 0.00% | Adventures of Bayou Billy, The (Europe) |
| 0.00% | 0.00% | Adventures of Lex and Grim, The (World) (Aftermarket) (Unl) |
| 0.00% | 0.00% | Adventures of Lolo (Europe) |
| 0.00% | 0.00% | Adventures of Lolo (Europe) (Virtual Console) |
| 0.00% | 0.00% | Adventures of Lolo 2 (Europe) |
| 0.00% | 0.00% | Adventures of Lolo 2 (Europe) (Virtual Console) |
| 0.00% | 0.00% | Adventures of Lolo 3 (Europe) |
| 0.00% | 0.00% | Adventures of Rad Gravity, The (Europe) |
| 0.00% | 0.00% | Air Fortress (Europe) |
| 0.00% | 0.00% | Airwolf (Europe) (Acclaim) |
| 0.00% | 0.00% | Aladdin (Europe) |
| 0.00% | 0.00% | Alfred Chicken (Europe) |
| 0.00% | 0.00% | Alien 3 (Europe) |
| 0.00% | 0.00% | Alpha Mission (Europe) |
| 0.00% | 0.00% | Anticipation (Europe) |
| 0.00% | 0.00% | Arch Rivals - A Basketbrawl! (Europe) |
| 0.00% | 0.00% | Asterix (Europe) (En,Fr,De,Es,It) |
| 0.00% | 0.00% | Astyanax (Europe) |
| 0.00% | 0.00% | Athletic World (Europe) |
| 0.00% | 0.00% | Attack of the Killer Tomatoes (Europe) |
| 0.00% | 0.00% | Aussie Rules Footy (Australia) |
| 0.00% | 0.00% | Bad Dudes vs. Dragon Ninja (Europe) |
| 0.00% | 0.00% | Balloon Fight (Europe) |
| 0.00% | 0.00% | Banana Prince (Germany) |
| 0.00% | 0.00% | Barbie (Europe) |
| 0.00% | 0.00% | Barker Bill's Trick Shooting (Europe) |
| 0.00% | 0.00% | Batman - Return of the Joker (Europe) |
| 0.00% | 0.00% | Batman - The Video Game (Europe) |
| 0.00% | 0.00% | Batman Returns (Europe) |
| 0.00% | 0.00% | Batman Returns (Europe) (Beta) |
| 0.00% | 0.00% | Battle of Olympus, The (Europe) |
| 0.00% | 0.00% | Battleship (Europe) (En,Fr,De,Es) |
| 0.00% | 0.00% | Battletoads (Europe) |
| 0.00% | 0.00% | Battletoads-Double Dragon (Europe) |
| 0.00% | 0.00% | Beauty and the Beast (Europe) |

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

