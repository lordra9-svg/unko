use unko_snes::spc700::{Flags, Spc700, SpcBus};

struct Ram {
    data: Vec<u8>,
}

impl Ram {
    fn new() -> Self {
        Self {
            data: vec![0; 0x1_0000],
        }
    }

    fn load(&mut self, address: u16, bytes: &[u8]) {
        self.data[address as usize..address as usize + bytes.len()].copy_from_slice(bytes);
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

fn setup(code: &[u8]) -> (Spc700, Ram) {
    let mut cpu = Spc700::new();
    cpu.pc = 0x0200;
    cpu.sp = 0xEF;
    cpu.flags = Flags::default();
    let mut ram = Ram::new();
    ram.load(0x0200, code);
    (cpu, ram)
}

fn step(cpu: &mut Spc700, ram: &mut Ram) -> u32 {
    let start = cpu.cycles;
    cpu.step(ram);
    (cpu.cycles - start) as u32
}

#[test]
fn lda_immediate() {
    let (mut cpu, mut ram) = setup(&[0xE8, 0x42]);
    assert_eq!(step(&mut cpu, &mut ram), 2);
    assert_eq!(cpu.a, 0x42);
    assert!(!cpu.flags.zero);
    assert!(!cpu.flags.negative);
}

#[test]
fn sta_direct() {
    let (mut cpu, mut ram) = setup(&[0xC4, 0x10]);
    cpu.a = 0x5A;
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(ram.data[0x10], 0x5A);
}

#[test]
fn adc_and_sbc() {
    let (mut cpu, mut ram) = setup(&[0x88, 0x01, 0xA8, 0x01]);
    cpu.a = 0x09;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x0A);
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x08);
}

#[test]
fn adc_sets_half_carry_and_overflow() {
    let (mut cpu, mut ram) = setup(&[0x88, 0x01]);
    cpu.a = 0x7F;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.flags.overflow);
    assert!(cpu.flags.negative);
    assert!(cpu.flags.half_carry);
}

#[test]
fn direct_direct_move() {
    let (mut cpu, mut ram) = setup(&[0xFA, 0x10, 0x20]);
    ram.data[0x10] = 0x77;
    assert_eq!(step(&mut cpu, &mut ram), 5);
    assert_eq!(ram.data[0x20], 0x77);
}

#[test]
fn inc_direct() {
    let (mut cpu, mut ram) = setup(&[0xAB, 0x10]);
    ram.data[0x10] = 0xFE;
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(ram.data[0x10], 0xFF);
}

#[test]
fn asl_accumulator() {
    let (mut cpu, mut ram) = setup(&[0x1C]);
    cpu.a = 0x81;
    assert_eq!(step(&mut cpu, &mut ram), 2);
    assert_eq!(cpu.a, 0x02);
    assert!(cpu.flags.carry);
}

#[test]
fn branch_taken_and_not_taken() {
    let (mut cpu, mut ram) = setup(&[0xF0, 0x02, 0x00, 0x00]);
    cpu.flags.zero = true;
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.pc, 0x0204);

    let (mut cpu, mut ram) = setup(&[0xF0, 0x02, 0x00, 0x00]);
    cpu.flags.zero = false;
    assert_eq!(step(&mut cpu, &mut ram), 2);
    assert_eq!(cpu.pc, 0x0202);
}

#[test]
fn jump_absolute() {
    let (mut cpu, mut ram) = setup(&[0x5F, 0x00, 0x30]);
    assert_eq!(step(&mut cpu, &mut ram), 3);
    assert_eq!(cpu.pc, 0x3000);
}

#[test]
fn call_and_return() {
    let (mut cpu, mut ram) = setup(&[0x3F, 0x00, 0x30]);
    ram.load(0x3000, &[0x6F]);
    assert_eq!(step(&mut cpu, &mut ram), 8);
    assert_eq!(cpu.pc, 0x3000);
    assert_eq!(cpu.sp, 0xED);
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.pc, 0x0203);
    assert_eq!(cpu.sp, 0xEF);
}

#[test]
fn push_and_pop() {
    let (mut cpu, mut ram) = setup(&[0x2D, 0xAE]);
    cpu.a = 0x42;
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.sp, 0xEE);
    assert_eq!(ram.data[0x01EF], 0x42);
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.sp, 0xEF);
}

#[test]
fn addw_ya_direct() {
    let (mut cpu, mut ram) = setup(&[0x7A, 0x10]);
    cpu.a = 0x34;
    cpu.y = 0x12;
    ram.data[0x10] = 0x78;
    ram.data[0x11] = 0x56;
    assert_eq!(step(&mut cpu, &mut ram), 5);
    assert_eq!(cpu.ya(), 0x68AC);
}

#[test]
fn daa_and_das() {
    let (mut cpu, mut ram) = setup(&[0xDF]);
    cpu.a = 0x9A;
    cpu.flags.carry = false;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.carry);

    let (mut cpu, mut ram) = setup(&[0xBE]);
    cpu.a = 0x00;
    cpu.flags.carry = true;
    cpu.flags.half_carry = false;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0xFA);
}

#[test]
fn mul_and_div() {
    let (mut cpu, mut ram) = setup(&[0xCF]);
    cpu.a = 0x10;
    cpu.y = 0x02;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.ya(), 0x0020);

    let (mut cpu, mut ram) = setup(&[0x9E]);
    cpu.a = 0x10;
    cpu.y = 0x00;
    cpu.x = 0x04;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x04);
    assert_eq!(cpu.y, 0x00);
}

#[test]
fn test_and_set_bits() {
    let (mut cpu, mut ram) = setup(&[0x0E, 0x00, 0x10]);
    cpu.a = 0x0F;
    ram.data[0x1000] = 0xF0;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x1000], 0xFF);
}

#[test]
fn psw_round_trip() {
    let flags = Flags {
        carry: true,
        zero: true,
        irq_disable: false,
        half_carry: true,
        break_flag: false,
        page: true,
        overflow: false,
        negative: true,
    };
    assert_eq!(Flags::from_byte(flags.to_byte()), flags);
}

#[test]
fn spc700_bit_logic() {
    // AND1 C,/membit with clear bit must clear carry.
    let (mut cpu, mut ram) = setup(&[0x6A, 0x10, 0x20]);
    ram.data[0x10] = 0x02;
    cpu.flags.carry = true;
    step(&mut cpu, &mut ram);
    assert!(!cpu.flags.carry);

    // OR1 C,membit with set bit must set carry.
    let (mut cpu, mut ram) = setup(&[0x0A, 0x10, 0x20]);
    ram.data[0x10] = 0x02;
    cpu.flags.carry = false;
    step(&mut cpu, &mut ram);
    assert!(cpu.flags.carry);

    // ST1 mem.bit writes carry.
    let (mut cpu, mut ram) = setup(&[0xCA, 0x10, 0x20]);
    cpu.flags.carry = true;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x10] & 0x02, 0x02);
}
