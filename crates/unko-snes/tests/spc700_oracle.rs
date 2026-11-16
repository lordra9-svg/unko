use spc700::{HasAddressBus, Processor, ProgramStatusWord};
use unko_snes::spc700::{Flags, Spc700, SpcBus};

const RAM_SIZE: usize = 0x1_0000;

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

impl SpcBus for Ram {
    fn read(&mut self, address: u16) -> u8 {
        self.data[address as usize]
    }

    fn write(&mut self, address: u16, value: u8) {
        self.data[address as usize] = value;
    }
}

struct OracleRam {
    data: Vec<u8>,
}

impl OracleRam {
    fn new() -> Self {
        Self {
            data: vec![0; RAM_SIZE],
        }
    }
}

impl HasAddressBus for OracleRam {
    fn io(&mut self) {}

    fn read(&mut self, address: usize) -> u8 {
        self.data[address & 0xFFFF]
    }

    fn write(&mut self, address: usize, value: u8) {
        self.data[address & 0xFFFF] = value;
    }
}

#[derive(Clone, Copy)]
struct State {
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    psw: u8,
}

impl State {
    fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xEF,
            psw: 0,
        }
    }
}

fn compare(label: &str, code: &[u8], state: State, patches: &[(u16, u8)]) {
    let mut memory = vec![0; RAM_SIZE];
    memory[0x0200..0x0200 + code.len()].copy_from_slice(code);
    for &(address, value) in patches {
        memory[address as usize] = value;
    }

    let mut ours = Spc700::new();
    ours.a = state.a;
    ours.x = state.x;
    ours.y = state.y;
    ours.sp = state.sp;
    ours.flags = Flags::from_byte(state.psw);
    ours.pc = 0x0200;
    let mut ours_bus = Ram::new();
    ours_bus.data.copy_from_slice(&memory);
    ours.step(&mut ours_bus);

    let mut oracle = Processor {
        a: state.a,
        x: state.x,
        y: state.y,
        sp: state.sp,
        pc: 0x0200,
        psw: ProgramStatusWord::from_byte(state.psw),
    };
    let mut oracle_bus = OracleRam::new();
    oracle_bus.data.copy_from_slice(&memory);
    oracle.step(&mut oracle_bus);

    assert_eq!(ours.a, oracle.a, "{label}: A");
    assert_eq!(ours.x, oracle.x, "{label}: X");
    assert_eq!(ours.y, oracle.y, "{label}: Y");
    assert_eq!(ours.sp, oracle.sp, "{label}: SP");
    assert_eq!(ours.pc, oracle.pc, "{label}: PC");
    assert_eq!(
        ours.psw(),
        oracle.psw.to_byte(),
        "{label}: PSW ours={:02X} oracle={:02X}",
        ours.psw(),
        oracle.psw.to_byte()
    );
    for (address, (ours, oracle)) in ours_bus.data.iter().zip(oracle_bus.data.iter()).enumerate() {
        if ours != oracle {
            panic!("{label}: memory at {address:04X}: ours {ours:02X} oracle {oracle:02X}");
        }
    }
}

#[test]
fn oracle_basic_ops() {
    let state = State::new();
    compare("LDA #", &[0xE8, 0x42], state, &[]);
    compare("ORA #", &[0x08, 0x0F], State { a: 0x50, ..state }, &[]);
    compare("AND #", &[0x28, 0x0F], State { a: 0x5A, ..state }, &[]);
    compare("EOR #", &[0x48, 0xFF], State { a: 0x5A, ..state }, &[]);
    compare("ADC #", &[0x88, 0x01], State { a: 0x09, ..state }, &[]);
    compare("SBC #", &[0xA8, 0x01], State { a: 0x0A, ..state }, &[]);
    compare("CMP #", &[0x68, 0x10], State { a: 0x20, ..state }, &[]);
    compare("XCN", &[0x9F], State { a: 0x12, ..state }, &[]);
}

#[test]
fn oracle_memory_ops() {
    let state = State::new();
    compare("STA dp", &[0xC4, 0x10], State { a: 0x5A, ..state }, &[]);
    compare("LDA dp", &[0xE4, 0x10], state, &[(0x10, 0x77)]);
    compare("MOV dp,dp", &[0xFA, 0x10, 0x20], state, &[(0x10, 0x77)]);
    compare("MOV dp,#", &[0x8F, 0x42, 0x10], state, &[]);
    compare("INC dp", &[0xAB, 0x10], state, &[(0x10, 0xFE)]);
    compare("DEC dp", &[0x8B, 0x10], state, &[(0x10, 0x01)]);
    compare("ASL dp", &[0x0B, 0x10], state, &[(0x10, 0x81)]);
    compare("ROL dp", &[0x2B, 0x10], state, &[(0x10, 0x81)]);
    compare("LSR dp", &[0x4B, 0x10], state, &[(0x10, 0x01)]);
    compare("ROR dp", &[0x6B, 0x10], state, &[(0x10, 0x01)]);
}

#[test]
fn oracle_absolute_and_indexed() {
    let state = State {
        x: 0x03,
        y: 0x04,
        ..State::new()
    };
    compare("LDA abs", &[0xE5, 0x00, 0x10], state, &[(0x1000, 0x55)]);
    compare("LDA abs,X", &[0xF5, 0x00, 0x10], state, &[(0x1003, 0x55)]);
    compare("LDA abs,Y", &[0xF6, 0x00, 0x10], state, &[(0x1004, 0x55)]);
    compare("STA abs,X", &[0xD5, 0x00, 0x10], state, &[]);
    compare(
        "LDA (dp,X)",
        &[0xE7, 0x10],
        state,
        &[(0x13, 0x00), (0x14, 0x20), (0x2000, 0x66)],
    );
    compare(
        "LDA (dp),Y",
        &[0xF7, 0x10],
        state,
        &[(0x10, 0x00), (0x11, 0x20), (0x2004, 0x66)],
    );
}

#[test]
fn oracle_branches_and_calls() {
    let state = State::new();
    compare(
        "BEQ taken",
        &[0xF0, 0x02, 0x00, 0x00],
        State { psw: 0x02, ..state },
        &[],
    );
    compare("BNE taken", &[0xD0, 0x02, 0x00, 0x00], state, &[]);
    compare("JMP abs", &[0x5F, 0x00, 0x30], state, &[]);
    compare("CALL abs", &[0x3F, 0x00, 0x30], state, &[(0x3000, 0x6F)]);
    compare(
        "RTS",
        &[0x6F],
        State { sp: 0xED, ..state },
        &[(0x01EE, 0x00), (0x01EF, 0x02)],
    );
}

#[test]
fn oracle_stack_and_flags() {
    let state = State::new();
    compare("PUSH A", &[0x2D], State { a: 0x42, ..state }, &[]);
    compare(
        "POP A",
        &[0xAE],
        State { sp: 0xEE, ..state },
        &[(0x01EF, 0x42)],
    );
    compare("PUSH P", &[0x0D], State { psw: 0xA5, ..state }, &[]);
    compare(
        "POP P",
        &[0x8E],
        State { sp: 0xEE, ..state },
        &[(0x01EF, 0xA5)],
    );
    compare("CLRC", &[0x60], State { psw: 0x01, ..state }, &[]);
    compare("SETC", &[0x80], state, &[]);
    compare("CLRP", &[0x20], State { psw: 0x20, ..state }, &[]);
    compare("SETP", &[0x40], state, &[]);
}

#[test]
fn oracle_word_ops() {
    let state = State::new();
    compare(
        "ADDW YA,dp",
        &[0x7A, 0x10],
        State {
            a: 0x34,
            y: 0x12,
            ..state
        },
        &[(0x10, 0x78), (0x11, 0x56)],
    );
    compare(
        "SUBW YA,dp",
        &[0x9A, 0x10],
        State {
            a: 0xAC,
            y: 0x68,
            ..state
        },
        &[(0x10, 0x78), (0x11, 0x56)],
    );
    compare(
        "CMPW YA,dp",
        &[0x5A, 0x10],
        State {
            a: 0x34,
            y: 0x12,
            ..state
        },
        &[(0x10, 0x78), (0x11, 0x56)],
    );
    compare(
        "MOVW dp,YA",
        &[0xDA, 0x10],
        State {
            a: 0x34,
            y: 0x12,
            ..state
        },
        &[],
    );
}

#[test]
fn oracle_arithmetic() {
    let state = State::new();
    compare(
        "MUL YA",
        &[0xCF],
        State {
            a: 0x10,
            y: 0x02,
            ..state
        },
        &[],
    );
    compare(
        "DIV YA,X",
        &[0x9E],
        State {
            a: 0x10,
            y: 0x00,
            x: 0x04,
            ..state
        },
        &[],
    );
    compare("DAA", &[0xDF], State { a: 0x9A, ..state }, &[]);
    compare(
        "DAS",
        &[0xBE],
        State {
            a: 0x00,
            psw: 0x01,
            ..state
        },
        &[],
    );
    compare(
        "TSET",
        &[0x0E, 0x00, 0x10],
        State { a: 0x0F, ..state },
        &[(0x1000, 0xF0)],
    );
    compare(
        "TCLR",
        &[0x4E, 0x00, 0x10],
        State { a: 0x0F, ..state },
        &[(0x1000, 0xFF)],
    );
}

#[test]
fn oracle_all_opcodes() {
    for opcode in 0u8..=0xFF {
        if opcode == 0xEF || opcode == 0xFF {
            continue; // WAIT/STOP are handled by the system, not the oracle step.
        }
        if opcode == 0x6A {
            // spc700 0.1.0 computes AND1 C,/membit as !(bit & C) instead of
            // C & !bit; the ares behaviour is covered by `spc700_bit_logic`.
            continue;
        }
        let code = [opcode, 0x10, 0x20, 0x30, 0x40];
        let state = State {
            a: 0x12,
            x: 0x03,
            y: 0x04,
            sp: 0xEF,
            psw: 0,
        };
        let patches = [(0xFFDE, 0x00), (0xFFDF, 0x02)];
        compare(&format!("opcode {opcode:02X}"), &code, state, &patches);
    }
}
