use unko_snes::cpu::{Cpu, CpuBus, Flags};
use w65c816::{AddressType, Signals, System, CPU as OracleCpu};

const RAM_SIZE: usize = 0x40_0000;

#[derive(Clone, Copy)]
struct State {
    emulation: bool,
    flags: Flags,
    a: u16,
    x: u16,
    y: u16,
    s: u16,
    d: u16,
    pbr: u8,
    dbr: u8,
}

impl State {
    fn native8() -> Self {
        Self {
            emulation: false,
            flags: Flags::default(),
            a: 0,
            x: 0,
            y: 0,
            s: 0x01FF,
            d: 0,
            pbr: 0,
            dbr: 0,
        }
    }

    fn native16() -> Self {
        Self {
            flags: Flags {
                mem_8: false,
                index_8: false,
                ..Flags::default()
            },
            ..Self::native8()
        }
    }

    fn emulation() -> Self {
        Self {
            emulation: true,
            ..Self::native8()
        }
    }
}

struct Ram {
    data: Vec<u8>,
}

impl Ram {
    fn new() -> Self {
        Self {
            data: vec![0; RAM_SIZE],
        }
    }
}

impl CpuBus for Ram {
    fn read(&mut self, address: u32) -> u8 {
        let index = address as usize % self.data.len();
        self.data[index]
    }

    fn write(&mut self, address: u32, value: u8) {
        let index = address as usize % self.data.len();
        self.data[index] = value;
    }
}

struct OracleRam {
    data: Vec<u8>,
    reset_pending: bool,
}

impl System for OracleRam {
    fn read(&mut self, address: u32, _kind: AddressType, _signals: &Signals) -> u8 {
        let index = address as usize % self.data.len();
        self.data[index]
    }

    fn write(&mut self, address: u32, value: u8, _kind: AddressType, _signals: &Signals) {
        let index = address as usize % self.data.len();
        self.data[index] = value;
    }

    fn res(&mut self) -> bool {
        let pending = self.reset_pending;
        self.reset_pending = false;
        pending
    }
}

fn load(data: &mut [u8], address: u32, bytes: &[u8]) {
    data[address as usize..address as usize + bytes.len()].copy_from_slice(bytes);
}

fn setup_ours(state: State, memory: &[u8]) -> (Cpu, Ram) {
    let mut cpu = Cpu::new();
    cpu.emulation = state.emulation;
    cpu.flags = state.flags;
    cpu.a = state.a;
    cpu.x = state.x;
    cpu.y = state.y;
    cpu.s = state.s;
    cpu.d = state.d;
    cpu.pbr = state.pbr;
    cpu.dbr = state.dbr;
    cpu.pc = 0x8000;
    let mut bus = Ram::new();
    bus.data[..memory.len()].copy_from_slice(memory);
    (cpu, bus)
}

fn setup_oracle(state: State, memory: &[u8]) -> (OracleCpu, OracleRam) {
    let mut cpu = OracleCpu::new();
    let mut bus = OracleRam {
        data: vec![0; RAM_SIZE],
        reset_pending: true,
    };
    bus.data[..memory.len()].copy_from_slice(memory);
    bus.data[0xFFFC] = 0x00;
    bus.data[0xFFFD] = 0x80;
    for _ in 0..7 {
        cpu.cycle(&mut bus);
    }
    cpu.set_e(state.emulation);
    cpu.set_p(state.flags_to_p());
    cpu.set_acc(state.a);
    cpu.set_x(state.x);
    cpu.set_y(state.y);
    cpu.set_s(state.s);
    cpu.set_d(state.d);
    cpu.set_pbr(state.pbr);
    cpu.set_dbr(state.dbr);
    cpu.set_pc(0x8000);
    (cpu, bus)
}

impl State {
    fn flags_to_p(self) -> u8 {
        (self.flags.carry as u8)
            | ((self.flags.zero as u8) << 1)
            | ((self.flags.irq_disable as u8) << 2)
            | ((self.flags.decimal as u8) << 3)
            | ((self.flags.index_8 as u8) << 4)
            | ((self.flags.mem_8 as u8) << 5)
            | ((self.flags.overflow as u8) << 6)
            | ((self.flags.negative as u8) << 7)
    }
}

fn compare(label: &str, code: &[u8], state: State, patches: &[(u32, u8)]) {
    let mut memory = vec![0; RAM_SIZE];
    load(&mut memory, 0x8000, code);
    for &(address, value) in patches {
        memory[address as usize] = value;
    }

    let (mut ours, mut ours_bus) = setup_ours(state, &memory);
    let (mut oracle, mut oracle_bus) = setup_oracle(state, &memory);
    ours_bus.data.copy_from_slice(&oracle_bus.data);

    let mut ours_cycles = 0;
    for _ in 0..100_000 {
        ours_cycles += ours.step(&mut ours_bus);
        if ours.stp {
            break;
        }
    }
    assert!(ours.stp, "{label}: our CPU did not reach STP");

    let mut oracle_cycles = 0;
    for _ in 0..100_000 {
        oracle.cycle(&mut oracle_bus);
        oracle_cycles += 1;
        if oracle.stopped() {
            break;
        }
    }
    assert!(oracle.stopped(), "{label}: oracle did not reach STP");

    assert_eq!(ours.a, oracle_acc(&oracle), "{label}: A");
    assert_eq!(ours.x, oracle.x(), "{label}: X");
    assert_eq!(ours.y, oracle.y(), "{label}: Y");
    assert_eq!(ours.s, oracle.s(), "{label}: S");
    assert_eq!(ours.d, oracle.d(), "{label}: D");
    assert_eq!(ours.pc, oracle.pc(), "{label}: PC");
    assert_eq!(ours.pbr, oracle.pbr(), "{label}: PBR");
    assert_eq!(ours.dbr, oracle_dbr(&oracle), "{label}: DBR");
    assert_eq!(ours.emulation, oracle.flags().emulation, "{label}: E");
    assert_eq!(ours.flags.carry, oracle.flags().carry, "{label}: carry");
    assert_eq!(ours.flags.zero, oracle.flags().zero, "{label}: zero");
    assert_eq!(
        ours.flags.irq_disable,
        oracle.flags().interrupt_disable,
        "{label}: irq disable"
    );
    assert_eq!(
        ours.flags.decimal,
        oracle.flags().decimal,
        "{label}: decimal"
    );
    assert_eq!(
        ours.flags.overflow,
        oracle.flags().overflow,
        "{label}: overflow"
    );
    assert_eq!(
        ours.flags.negative,
        oracle.flags().negative,
        "{label}: negative"
    );
    assert_eq!(
        ours.flags.mem_8,
        oracle.flags().mem_sel,
        "{label}: memory width"
    );
    assert_eq!(
        ours.flags.index_8,
        oracle.flags().index_sel,
        "{label}: index width"
    );
    assert_eq!(ours_cycles, oracle_cycles, "{label}: cycle count");
    if let Some((address, (ours, oracle))) = ours_bus
        .data
        .iter()
        .zip(oracle_bus.data.iter())
        .enumerate()
        .find(|(_, (ours, oracle))| ours != oracle)
    {
        panic!("{label}: memory at {address:06X}: ours {ours:02X} oracle {oracle:02X}");
    }
}

fn oracle_acc(cpu: &OracleCpu) -> u16 {
    (u16::from(cpu.b()) << 8) | u16::from(cpu.a())
}

fn oracle_dbr(cpu: &OracleCpu) -> u8 {
    cpu.dbr()
}

#[test]
fn oracle_basic_8_bit() {
    let code = [
        0xA9, 0x12, // LDA #$12
        0x69, 0x34, // ADC #$34
        0x85, 0x10, // STA $10
        0xA2, 0x05, // LDX #$05
        0xE8, // INX
        0xCA, // DEX
        0x48, // PHA
        0x68, // PLA
        0xAA, // TAX
        0xA8, // TAY
        0x8A, // TXA
        0x98, // TYA
        0xDB, // STP
    ];
    compare("basic 8-bit", &code, State::native8(), &[]);
}

#[test]
fn oracle_basic_16_bit() {
    let code = [
        0xC2, 0x30, // REP #$30
        0xA9, 0x34, 0x12, // LDA #$1234
        0x69, 0x78, 0x56, // ADC #$5678
        0x85, 0x10, // STA $10
        0xA2, 0xFF, 0x00, // LDX #$00FF
        0xA0, 0x00, 0x01, // LDY #$0100
        0xE8, // INX
        0x88, // DEY
        0x48, // PHA
        0x68, // PLA
        0xDB, // STP
    ];
    compare("basic 16-bit", &code, State::native8(), &[]);
}

#[test]
fn oracle_decimal_mode() {
    let code = [
        0xF8, // SED
        0x18, // CLC
        0xA9, 0x09, // LDA #$09
        0x69, 0x01, // ADC #$01
        0xA9, 0x99, // LDA #$99
        0x69, 0x01, // ADC #$01
        0xD8, // CLD
        0xDB, // STP
    ];
    compare("decimal", &code, State::native8(), &[]);
}

#[test]
fn oracle_memory_addressing() {
    let code = [
        0xA9, 0x11, // LDA #$11
        0x85, 0x10, // STA $10
        0xA9, 0x22, // LDA #$22
        0x95, 0x10, // STA $10,X
        0xA9, 0x33, // LDA #$33
        0x8D, 0x00, 0x20, // STA $2000
        0xA9, 0x44, // LDA #$44
        0x9D, 0x00, 0x20, // STA $2000,X
        0xA9, 0x55, // LDA #$55
        0x91, 0x30, // STA ($30),Y
        0xA9, 0x66, // LDA #$66
        0x87, 0x40, // STA [$40]
        0xDB, // STP
    ];
    let mut state = State::native8();
    state.d = 0x0020;
    state.dbr = 0x01;
    state.x = 0x0002;
    state.y = 0x0003;
    let patches = [
        (0x0050, 0x00), // pointer for ($30),Y -> $0100
        (0x0051, 0x01),
        (0x0060, 0x00), // pointer for [$40] -> $010000
        (0x0061, 0x00),
        (0x0062, 0x01),
    ];
    compare("memory addressing", &code, state, &patches);
}

#[test]
fn oracle_stack_and_flags() {
    let code = [
        0xF4, 0x34, 0x12, // PEA $1234
        0xD4, 0x10, // PEI ($10)
        0x62, 0x00, 0x00, // PER $0000
        0x08, // PHP
        0x28, // PLP
        0x48, // PHA
        0x68, // PLA
        0x5A, // PHY
        0x7A, // PLY
        0xDA, // PHX
        0xFA, // PLX
        0x0B, // PHD
        0x2B, // PLD
        0x4B, // PHK
        0x8B, // PHB
        0xAB, // PLB
        0xDB, // STP
    ];
    let mut state = State::native8();
    state.a = 0x0034;
    state.x = 0x0078;
    state.y = 0x00BC;
    state.s = 0x01F0;
    state.d = 0x0020;
    let patches = [
        (0x0030, 0x78), // PEI ($10) -> $5678
        (0x0031, 0x56),
    ];
    compare("stack and flags", &code, state, &patches);
}

#[test]
fn oracle_branches_and_calls() {
    let code = [
        0xA9, 0x00, // LDA #$00
        0xF0, 0x03, // BEQ +3
        0xEA, // NOP
        0xA9, 0x01, // LDA #$01
        0x20, 0x00, 0x90, // JSR $9000
        0xDB, // STP
    ];
    let mut state = State::native8();
    state.flags.zero = true;
    let patches = [
        (0x9000, 0xA9), // LDA #$02
        (0x9001, 0x02),
        (0x9002, 0x60), // RTS
    ];
    compare("branches and calls", &code, state, &patches);
}

#[test]
fn oracle_block_move() {
    let code = [
        0x54, 0x00, 0x01, // MVN $00,$01
        0xDB, // STP
    ];
    let mut state = State::native16();
    state.a = 2;
    state.x = 0x0010;
    state.y = 0x0020;
    let patches = [(0x01_0010, 0xAA), (0x01_0011, 0xBB)];
    compare("block move", &code, state, &patches);
}

#[test]
fn oracle_exchange_and_transfers() {
    let code = [
        0xEB, // XBA
        0xFB, // XCE
        0x9B, // TXY
        0xBB, // TYX
        0xBA, // TSX
        0x9A, // TXS
        0x1B, // TCS
        0x3B, // TSC
        0x5B, // TCD
        0x7B, // TDC
        0xDB, // STP
    ];
    let mut state = State::native16();
    state.a = 0x1234;
    state.x = 0x5678;
    state.y = 0x9ABC;
    state.s = 0x01F0;
    state.flags.carry = true;
    compare("exchange and transfers", &code, state, &[]);
}

#[test]
fn oracle_emulation_mode() {
    let code = [
        0xA9, 0x12, // LDA #$12
        0x85, 0x10, // STA $10
        0xA2, 0x05, // LDX #$05
        0xE8, // INX
        0x18, // CLC
        0xFB, // XCE -> should stay in emulation (carry was clear)
        0xDB, // STP
    ];
    compare("emulation mode", &code, State::emulation(), &[]);
}

#[test]
fn oracle_read_addressing_modes() {
    const MODES: &[(&str, u8, &[u8])] = &[
        ("(dp,X)", 0x01, &[0x12]),
        ("sr,S", 0x03, &[0x10]),
        ("dp", 0x05, &[0x10]),
        ("[dp]", 0x07, &[0x40]),
        ("#imm", 0x09, &[0x42]),
        ("abs", 0x0D, &[0x00, 0x20]),
        ("long", 0x0F, &[0x00, 0x20, 0x02]),
        ("(dp),Y", 0x11, &[0x50]),
        ("(dp)", 0x12, &[0x60]),
        ("(sr,S),Y", 0x13, &[0x20]),
        ("dp,X", 0x15, &[0x11]),
        ("[dp],Y", 0x17, &[0x40]),
        ("abs,Y", 0x19, &[0x00, 0x20]),
        ("abs,X", 0x1D, &[0x00, 0x20]),
        ("long,X", 0x1F, &[0x00, 0x20, 0x02]),
    ];
    let patches = [
        (0x0030, 0x11),
        (0x0034, 0x22),
        (0x0035, 0x00),
        (0x0036, 0x24),
        (0x0060, 0x00),
        (0x0061, 0x20),
        (0x0062, 0x02),
        (0x0070, 0x00),
        (0x0071, 0x23),
        (0x0080, 0x00),
        (0x0081, 0x21),
        (0x0200, 0x33),
        (0x0210, 0x00),
        (0x0211, 0x22),
        (0x012000, 0x44),
        (0x012003, 0x55),
        (0x012004, 0x88),
        (0x022000, 0x66),
        (0x022003, 0x77),
        (0x022004, 0xEE),
        (0x2304, 0xAA),
        (0x2100, 0xBB),
        (0x2204, 0xCC),
        (0x2400, 0xDD),
    ];
    let mut state = State::native8();
    state.a = 0x10;
    state.d = 0x0020;
    state.dbr = 0x01;
    state.x = 0x0003;
    state.y = 0x0004;
    state.flags.carry = true;

    for &(name, base, operands) in MODES {
        for offset in [0x00u8, 0x20, 0x40, 0x60, 0xA0, 0xC0, 0xE0] {
            let mut code = vec![base + offset];
            code.extend_from_slice(operands);
            code.push(0xDB);
            compare(
                &format!("read {name} opcode {:02X}", base + offset),
                &code,
                state,
                &patches,
            );
        }
    }
}

#[test]
fn oracle_write_addressing_modes() {
    const CASES: &[(&str, u8, &[u8])] = &[
        ("STA (dp,X)", 0x81, &[0x12]),
        ("STA sr,S", 0x83, &[0x10]),
        ("STA dp", 0x85, &[0x10]),
        ("STA [dp]", 0x87, &[0x40]),
        ("STA abs", 0x8D, &[0x00, 0x20]),
        ("STA long", 0x8F, &[0x00, 0x20, 0x02]),
        ("STA (dp),Y", 0x91, &[0x50]),
        ("STA (dp)", 0x92, &[0x60]),
        ("STA (sr,S),Y", 0x93, &[0x20]),
        ("STA dp,X", 0x95, &[0x11]),
        ("STA [dp],Y", 0x97, &[0x40]),
        ("STA abs,Y", 0x99, &[0x00, 0x20]),
        ("STA abs,X", 0x9D, &[0x00, 0x20]),
        ("STA long,X", 0x9F, &[0x00, 0x20, 0x02]),
        ("STX dp", 0x86, &[0x10]),
        ("STX abs", 0x8E, &[0x00, 0x20]),
        ("STX dp,Y", 0x96, &[0x11]),
        ("STY dp", 0x84, &[0x10]),
        ("STY abs", 0x8C, &[0x00, 0x20]),
        ("STY dp,X", 0x94, &[0x11]),
        ("STZ dp", 0x64, &[0x10]),
        ("STZ dp,X", 0x74, &[0x11]),
        ("STZ abs", 0x9C, &[0x00, 0x20]),
        ("STZ abs,X", 0x9E, &[0x00, 0x20]),
    ];
    let patches = [
        (0x0035, 0x00),
        (0x0036, 0x24),
        (0x0060, 0x00),
        (0x0061, 0x20),
        (0x0062, 0x02),
        (0x0070, 0x00),
        (0x0071, 0x23),
        (0x0080, 0x00),
        (0x0081, 0x21),
        (0x0210, 0x00),
        (0x0211, 0x22),
    ];
    let mut state = State::native8();
    state.a = 0x42;
    state.x = 0x43;
    state.y = 0x44;
    state.d = 0x0020;
    state.dbr = 0x01;
    state.flags.carry = true;

    for &(name, opcode, operands) in CASES {
        let mut code = vec![opcode];
        code.extend_from_slice(operands);
        code.push(0xDB);
        compare(&format!("write {name}"), &code, state, &patches);
    }
}

#[test]
fn oracle_modify_addressing_modes() {
    const CASES: &[(&str, u8, &[u8])] = &[
        ("ASL A", 0x0A, &[]),
        ("ASL dp", 0x06, &[0x10]),
        ("ASL dp,X", 0x16, &[0x11]),
        ("ASL abs", 0x0E, &[0x00, 0x20]),
        ("ASL abs,X", 0x1E, &[0x00, 0x20]),
        ("LSR A", 0x4A, &[]),
        ("LSR dp", 0x46, &[0x10]),
        ("LSR dp,X", 0x56, &[0x11]),
        ("LSR abs", 0x4E, &[0x00, 0x20]),
        ("LSR abs,X", 0x5E, &[0x00, 0x20]),
        ("ROL A", 0x2A, &[]),
        ("ROL dp", 0x26, &[0x10]),
        ("ROL dp,X", 0x36, &[0x11]),
        ("ROL abs", 0x2E, &[0x00, 0x20]),
        ("ROL abs,X", 0x3E, &[0x00, 0x20]),
        ("ROR A", 0x6A, &[]),
        ("ROR dp", 0x66, &[0x10]),
        ("ROR dp,X", 0x76, &[0x11]),
        ("ROR abs", 0x6E, &[0x00, 0x20]),
        ("ROR abs,X", 0x7E, &[0x00, 0x20]),
        ("INC A", 0x1A, &[]),
        ("INC dp", 0xE6, &[0x10]),
        ("INC dp,X", 0xF6, &[0x11]),
        ("INC abs", 0xEE, &[0x00, 0x20]),
        ("INC abs,X", 0xFE, &[0x00, 0x20]),
        ("DEC A", 0x3A, &[]),
        ("DEC dp", 0xC6, &[0x10]),
        ("DEC dp,X", 0xD6, &[0x11]),
        ("DEC abs", 0xCE, &[0x00, 0x20]),
        ("DEC abs,X", 0xDE, &[0x00, 0x20]),
        ("TSB dp", 0x04, &[0x10]),
        ("TSB abs", 0x0C, &[0x00, 0x20]),
        ("TRB dp", 0x14, &[0x10]),
        ("TRB abs", 0x1C, &[0x00, 0x20]),
    ];
    let patches = [
        (0x0030, 0x81),
        (0x0034, 0x40),
        (0x012000, 0x20),
        (0x012003, 0x10),
    ];
    let mut state = State::native8();
    state.a = 0x0F;
    state.d = 0x0020;
    state.dbr = 0x01;
    state.x = 0x0003;
    state.flags.carry = true;

    for &(name, opcode, operands) in CASES {
        if opcode == 0x0A
            || opcode == 0x4A
            || opcode == 0x2A
            || opcode == 0x6A
            || opcode == 0x1A
            || opcode == 0x3A
        {
            state.a = 0x81;
        }
        let mut code = vec![opcode];
        code.extend_from_slice(operands);
        code.push(0xDB);
        compare(&format!("modify {name}"), &code, state, &patches);
    }
}

type BranchCase = (&'static str, u8, bool, fn(&mut State));

#[test]
fn oracle_branches() {
    let cases: &[BranchCase] = &[
        ("BPL", 0x10, true, |s| s.flags.negative = false),
        ("BPL", 0x10, false, |s| s.flags.negative = true),
        ("BMI", 0x30, true, |s| s.flags.negative = true),
        ("BVC", 0x50, true, |s| s.flags.overflow = false),
        ("BVS", 0x70, true, |s| s.flags.overflow = true),
        ("BCC", 0x90, true, |s| s.flags.carry = false),
        ("BCS", 0xB0, true, |s| s.flags.carry = true),
        ("BNE", 0xD0, true, |s| s.flags.zero = false),
        ("BEQ", 0xF0, true, |s| s.flags.zero = true),
        ("BRA", 0x80, true, |_| {}),
    ];
    for &(name, opcode, taken, set) in cases {
        let mut state = State::native8();
        set(&mut state);
        let code = [opcode, 0x02, 0xEA, 0xEA, 0xDB];
        compare(&format!("branch {name} taken={taken}"), &code, state, &[]);
    }

    let mut state = State::native8();
    state.flags.zero = false;
    compare("BRL", &[0x82, 0x02, 0x00, 0xEA, 0xEA, 0xDB], state, &[]);
}

#[test]
fn oracle_jumps_and_calls() {
    let state = State::native8();
    compare("JMP abs", &[0x4C, 0x00, 0x90], state, &[(0x9000, 0xDB)]);
    compare(
        "JMP (abs)",
        &[0x6C, 0x10, 0x00],
        state,
        &[(0x0010, 0x00), (0x0011, 0x90), (0x9000, 0xDB)],
    );
    let mut indexed = state;
    indexed.x = 2;
    compare(
        "JMP (abs,X)",
        &[0x7C, 0x10, 0x00],
        indexed,
        &[(0x0012, 0x00), (0x0013, 0x90), (0x9000, 0xDB)],
    );
    compare(
        "JML long",
        &[0x5C, 0x00, 0x90, 0x02],
        state,
        &[(0x02_9000, 0xDB)],
    );
    compare(
        "JML [abs]",
        &[0xDC, 0x10, 0x00],
        state,
        &[
            (0x0010, 0x00),
            (0x0011, 0x90),
            (0x0012, 0x02),
            (0x02_9000, 0xDB),
        ],
    );
    compare(
        "JSR/JSL",
        &[0x20, 0x00, 0x90, 0xDB],
        state,
        &[(0x9000, 0x60)],
    );
}

#[test]
fn oracle_brk_and_rti() {
    // w65c816 0.1.17 always pulls a program bank byte in RTI, even in
    // emulation mode, so emulation RTI is covered by the ares-derived unit
    // test instead.
    let code = [0x00, 0xEA, 0xDB];
    let patches = [(0xFFE6, 0x00), (0xFFE7, 0x90), (0x9000, 0x40)];
    compare("BRK/RTI native", &code, State::native8(), &patches);
}

#[test]
fn oracle_cop_and_rti() {
    let code = [0x02, 0xEA, 0xDB];
    let patches = [(0xFFE4, 0x00), (0xFFE5, 0x90), (0x9000, 0x40)];
    compare("COP/RTI native", &code, State::native8(), &patches);
}

#[test]
fn oracle_16_bit_indexed_addressing() {
    let code = [
        0xBD, 0x00, 0x20, // LDA $2000,X (16-bit)
        0x9D, 0x00, 0x30, // STA $3000,X (16-bit)
        0xDB, // STP
    ];
    let mut state = State::native16();
    state.d = 0x0020;
    state.dbr = 0x01;
    state.x = 0x0100;
    let patches = [(0x012100, 0x34), (0x012101, 0x12)];
    compare("16-bit indexed", &code, state, &patches);
}

#[test]
fn oracle_decimal_16() {
    let code = [
        0xF8, // SED
        0x18, // CLC
        0xC2, 0x20, // REP #$20
        0xA9, 0x99, 0x00, // LDA #$0099
        0x69, 0x01, 0x00, // ADC #$0001
        0xD8, // CLD
        0xDB, // STP
    ];
    compare("decimal 16-bit", &code, State::native16(), &[]);
}
