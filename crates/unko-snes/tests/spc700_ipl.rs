use spc700::{HasAddressBus, Processor, ProgramStatusWord, IPL};
use unko_snes::spc700::{Spc700, SpcBus};

#[derive(Clone)]
struct Bus {
    ram: Vec<u8>,
    input: [u8; 4],
    output: [u8; 4],
    stage: u8,
}

impl Bus {
    fn new() -> Self {
        Self {
            ram: vec![0; 0x10000],
            input: [0; 4],
            output: [0; 4],
            stage: 0,
        }
    }

    fn write_port(&mut self, port: usize, value: u8) {
        self.output[port] = value;
        match (self.stage, port, value) {
            (0, 1, 0xBB) => {
                self.input = [0xCC, 0x01, 0x00, 0x05];
                self.stage = 1;
            }
            (1, 0, 0xCC) => {
                self.input = [0x00, 0x20, 0x00, 0x05];
                self.stage = 2;
            }
            (2, 0, 0x00) => {
                self.input = [0x01, 0xCD, 0x00, 0x05];
                self.stage = 3;
            }
            _ => {}
        }
    }
}

impl SpcBus for Bus {
    fn read(&mut self, address: u16) -> u8 {
        match address {
            0xFFC0..=0xFFFF => IPL[usize::from(address & 0x3F)],
            0xF4..=0xF7 => self.input[usize::from(address - 0xF4)],
            _ => self.ram[usize::from(address)],
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0xF4..=0xF7 => self.write_port(usize::from(address - 0xF4), value),
            _ => self.ram[usize::from(address)] = value,
        }
    }
}

impl HasAddressBus for Bus {
    fn io(&mut self) {}

    fn read(&mut self, address: usize) -> u8 {
        SpcBus::read(self, address as u16)
    }

    fn write(&mut self, address: usize, value: u8) {
        SpcBus::write(self, address as u16, value)
    }
}

#[test]
fn ipl_handshake_matches_oracle() {
    let mut ours = Spc700::new();
    ours.pc = 0xFFC0;
    let mut ours_bus = Bus::new();

    let mut oracle = Processor {
        a: 0,
        x: 0,
        y: 0,
        sp: 0xEF,
        pc: 0xFFC0,
        psw: ProgramStatusWord::from_byte(0),
    };
    let mut oracle_bus = Bus::new();

    for step in 0..5000 {
        ours.step(&mut ours_bus);
        oracle.step(&mut oracle_bus);
        if ours.pc != oracle.pc
            || ours.a != oracle.a
            || ours.x != oracle.x
            || ours.y != oracle.y
            || ours.sp != oracle.sp
            || ours.psw() != oracle.psw.to_byte()
        {
            panic!(
                "step {step}: ours PC={:04X} A={:02X} X={:02X} Y={:02X} SP={:02X} PSW={:02X} out={:02X?}; oracle PC={:04X} A={:02X} X={:02X} Y={:02X} SP={:02X} PSW={:02X} out={:02X?}",
                ours.pc, ours.a, ours.x, ours.y, ours.sp, ours.psw(), ours_bus.output,
                oracle.pc, oracle.a, oracle.x, oracle.y, oracle.sp, oracle.psw.to_byte(), oracle_bus.output
            );
        }
    }
}
