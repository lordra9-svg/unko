use unko_snes::cpu::{Cpu, CpuBus, Flags};

struct Ram {
    data: Vec<u8>,
}

impl Ram {
    fn new() -> Self {
        Self {
            data: vec![0; 0x100_0000],
        }
    }

    fn load(&mut self, address: u32, bytes: &[u8]) {
        self.data[address as usize..address as usize + bytes.len()].copy_from_slice(bytes);
    }
}

impl CpuBus for Ram {
    fn read(&mut self, address: u32) -> u8 {
        self.data[(address & 0x00FF_FFFF) as usize]
    }

    fn write(&mut self, address: u32, value: u8) {
        self.data[(address & 0x00FF_FFFF) as usize] = value;
    }
}

fn flags(mem_8: bool, index_8: bool) -> Flags {
    Flags {
        mem_8,
        index_8,
        ..Flags::default()
    }
}

fn setup(code: &[u8]) -> (Cpu, Ram) {
    let mut cpu = Cpu::new();
    cpu.emulation = false;
    cpu.flags = flags(true, true);
    cpu.pc = 0x8000;
    cpu.pbr = 0;
    cpu.dbr = 0;
    cpu.d = 0;
    cpu.s = 0x01FF;
    let mut ram = Ram::new();
    ram.load(0x8000, code);
    (cpu, ram)
}

fn step(cpu: &mut Cpu, ram: &mut Ram) -> u32 {
    let start = cpu.cycles;
    cpu.step(ram);
    (cpu.cycles - start) as u32
}

#[test]
fn lda_immediate_8() {
    let (mut cpu, mut ram) = setup(&[0xA9, 0x42, 0xDB]);
    assert_eq!(step(&mut cpu, &mut ram), 2);
    assert_eq!(cpu.a, 0x42);
    assert!(!cpu.flags.zero);
    assert!(!cpu.flags.negative);
    assert_eq!(cpu.pc, 0x8002);
}

#[test]
fn lda_immediate_16() {
    let (mut cpu, mut ram) = setup(&[0xA9, 0x34, 0x12, 0xDB]);
    cpu.flags = flags(false, true);
    assert_eq!(step(&mut cpu, &mut ram), 3);
    assert_eq!(cpu.a, 0x1234);
    assert!(!cpu.flags.zero);
    assert!(!cpu.flags.negative);
}

#[test]
fn lda_sets_zero_and_negative() {
    let (mut cpu, mut ram) = setup(&[0xA9, 0x00, 0xA9, 0x80, 0xDB]);
    step(&mut cpu, &mut ram);
    assert!(cpu.flags.zero);
    step(&mut cpu, &mut ram);
    assert!(cpu.flags.negative);
}

#[test]
fn sta_direct() {
    let (mut cpu, mut ram) = setup(&[0x85, 0x10, 0xDB]);
    cpu.a = 0x5A;
    assert_eq!(step(&mut cpu, &mut ram), 3);
    assert_eq!(ram.data[0x10], 0x5A);
    assert_eq!(cpu.pc, 0x8002);
}

#[test]
fn direct_page_emulation_wrap() {
    let (mut cpu, mut ram) = setup(&[0x85, 0x10, 0xDB]);
    cpu.emulation = true;
    cpu.flags = flags(true, true);
    cpu.d = 0x0100;
    cpu.a = 0x5A;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x0110], 0x5A);
}

#[test]
fn direct_page_offset_adds() {
    let (mut cpu, mut ram) = setup(&[0x85, 0x10, 0xDB]);
    cpu.d = 0x0101;
    cpu.a = 0x5A;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x0111], 0x5A);
}

#[test]
fn lda_absolute_indexed_page_cross_costs_a_cycle() {
    let (mut cpu, mut ram) = setup(&[0xBD, 0x00, 0x90, 0xDB]);
    cpu.x = 0x0100;
    ram.data[0x9100] = 0x77;
    assert_eq!(step(&mut cpu, &mut ram), 5);
    assert_eq!(cpu.a, 0x77);
}

#[test]
fn lda_absolute_indexed_no_page_cross() {
    let (mut cpu, mut ram) = setup(&[0xBD, 0x00, 0x90, 0xDB]);
    cpu.x = 0x0010;
    ram.data[0x9010] = 0x77;
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.a, 0x77);
}

#[test]
fn adc_binary_and_carry() {
    let (mut cpu, mut ram) = setup(&[0x69, 0x01, 0xDB]);
    cpu.a = 0xFF;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.carry);
    assert!(cpu.flags.zero);
}

#[test]
fn adc_decimal_8() {
    let (mut cpu, mut ram) = setup(&[0x69, 0x01, 0xDB]);
    cpu.a = 0x09;
    cpu.flags = flags(true, true);
    cpu.flags.decimal = true;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x10);
    assert!(!cpu.flags.carry);
}

#[test]
fn adc_decimal_8_overflow() {
    let (mut cpu, mut ram) = setup(&[0x69, 0x01, 0xDB]);
    cpu.a = 0x99;
    cpu.flags.decimal = true;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.carry);
}

#[test]
fn sbc_decimal_8() {
    let (mut cpu, mut ram) = setup(&[0xE9, 0x01, 0xDB]);
    cpu.a = 0x10;
    cpu.flags.decimal = true;
    cpu.flags.carry = true;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x09);
    assert!(cpu.flags.carry);
}

#[test]
fn sbc_decimal_8_underflow_wraps() {
    let (mut cpu, mut ram) = setup(&[0xE9, 0x01, 0xDB]);
    cpu.a = 0x00;
    cpu.flags.decimal = true;
    cpu.flags.carry = true;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x99);
    assert!(!cpu.flags.carry);
}

#[test]
fn adc_decimal_16() {
    let (mut cpu, mut ram) = setup(&[0x69, 0x01, 0x00, 0xDB]);
    cpu.flags = flags(false, true);
    cpu.flags.decimal = true;
    cpu.a = 0x0099;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x0100);
    assert!(!cpu.flags.carry);
}

#[test]
fn cmp_sets_flags_without_changing_accumulator() {
    let (mut cpu, mut ram) = setup(&[0xC9, 0x10, 0xDB]);
    cpu.a = 0x20;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x20);
    assert!(cpu.flags.carry);
    assert!(!cpu.flags.zero);
}

#[test]
fn bit_immediate_only_changes_zero() {
    let (mut cpu, mut ram) = setup(&[0x89, 0x40, 0xDB]);
    cpu.a = 0x40;
    cpu.flags.overflow = true;
    cpu.flags.negative = true;
    step(&mut cpu, &mut ram);
    assert!(!cpu.flags.zero);
    assert!(cpu.flags.overflow);
    assert!(cpu.flags.negative);
}

#[test]
fn bit_memory_changes_overflow_and_negative() {
    let (mut cpu, mut ram) = setup(&[0x24, 0x10, 0xDB]);
    cpu.a = 0x00;
    ram.data[0x10] = 0xC0;
    step(&mut cpu, &mut ram);
    assert!(cpu.flags.zero);
    assert!(cpu.flags.overflow);
    assert!(cpu.flags.negative);
}

#[test]
fn tsb_and_trb() {
    let (mut cpu, mut ram) = setup(&[0x04, 0x10, 0x14, 0x10, 0xDB]);
    cpu.a = 0x0F;
    ram.data[0x10] = 0xF0;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x10], 0xFF);
    assert!(cpu.flags.zero);
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x10], 0xF0);
}

#[test]
fn asl_implied_and_memory() {
    let (mut cpu, mut ram) = setup(&[0x0A, 0x06, 0x10, 0xDB]);
    cpu.a = 0x81;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x02);
    assert!(cpu.flags.carry);

    ram.data[0x10] = 0x40;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x10], 0x80);
    assert!(!cpu.flags.carry);
    assert!(cpu.flags.negative);
}

#[test]
fn inc_and_dec_memory() {
    let (mut cpu, mut ram) = setup(&[0xE6, 0x10, 0xC6, 0x10, 0xDB]);
    ram.data[0x10] = 0xFE;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x10], 0xFF);
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x10], 0xFE);
}

#[test]
fn push_and_pull_accumulator_8() {
    let (mut cpu, mut ram) = setup(&[0x48, 0x68, 0xDB]);
    cpu.a = 0x42;
    assert_eq!(step(&mut cpu, &mut ram), 3);
    assert_eq!(cpu.s, 0x01FE);
    assert_eq!(ram.data[0x01FF], 0x42);
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.s, 0x01FF);
}

#[test]
fn push_and_pull_accumulator_16() {
    let (mut cpu, mut ram) = setup(&[0x48, 0x68, 0xDB]);
    cpu.flags = flags(false, true);
    cpu.a = 0x1234;
    assert_eq!(step(&mut cpu, &mut ram), 4);
    assert_eq!(cpu.s, 0x01FD);
    assert_eq!(ram.data[0x01FF], 0x12);
    assert_eq!(ram.data[0x01FE], 0x34);
    assert_eq!(step(&mut cpu, &mut ram), 5);
    assert_eq!(cpu.a, 0x1234);
}

#[test]
fn transfer_accumulator_to_index_8() {
    let (mut cpu, mut ram) = setup(&[0xAA, 0xA8, 0xDB]);
    cpu.a = 0x1234;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.x, 0x34);
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.y, 0x34);
}

#[test]
fn transfer_accumulator_to_index_16() {
    let (mut cpu, mut ram) = setup(&[0xAA, 0xA8, 0xDB]);
    cpu.flags = flags(false, false);
    cpu.a = 0x1234;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.x, 0x1234);
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.y, 0x1234);
}

#[test]
fn index_to_accumulator_16() {
    let (mut cpu, mut ram) = setup(&[0x8A, 0x98, 0xDB]);
    cpu.flags = flags(false, false);
    cpu.x = 0x1234;
    cpu.y = 0x5678;
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x1234);
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x5678);
}

#[test]
fn exchange_ba() {
    let (mut cpu, mut ram) = setup(&[0xEB, 0xDB]);
    cpu.a = 0x1234;
    assert_eq!(step(&mut cpu, &mut ram), 3);
    assert_eq!(cpu.a, 0x3412);
}

#[test]
fn exchange_ce_enters_emulation() {
    let (mut cpu, mut ram) = setup(&[0xFB, 0xDB]);
    cpu.flags.carry = true;
    cpu.x = 0x1234;
    cpu.y = 0x5678;
    cpu.s = 0x02FF;
    step(&mut cpu, &mut ram);
    assert!(cpu.emulation);
    assert_eq!(cpu.x, 0x0034);
    assert_eq!(cpu.y, 0x0078);
    assert_eq!(cpu.s, 0x01FF);
    assert!(!cpu.flags.carry); // old E (false) moves into carry
}

#[test]
fn sep_and_rep() {
    let (mut cpu, mut ram) = setup(&[0xE2, 0x30, 0xC2, 0x20, 0xDB]);
    cpu.flags = flags(false, false);
    step(&mut cpu, &mut ram);
    assert!(cpu.flags.mem_8);
    assert!(cpu.flags.index_8);
    step(&mut cpu, &mut ram);
    assert!(!cpu.flags.mem_8);
    assert!(cpu.flags.index_8);
}

#[test]
fn branch_taken_and_not_taken() {
    let (mut cpu, mut ram) = setup(&[0xD0, 0x02, 0xEA, 0xEA, 0xDB]);
    cpu.flags.zero = false;
    assert_eq!(step(&mut cpu, &mut ram), 3);
    assert_eq!(cpu.pc, 0x8004);

    let (mut cpu, mut ram) = setup(&[0xD0, 0x02, 0xEA, 0xEA, 0xDB]);
    cpu.flags.zero = true;
    assert_eq!(step(&mut cpu, &mut ram), 2);
    assert_eq!(cpu.pc, 0x8002);
}

#[test]
fn branch_page_cross_in_emulation_costs_a_cycle() {
    let (mut cpu, mut ram) = setup(&[0xD0, 0x7F, 0xDB]);
    cpu.pc = 0x80FD;
    cpu.emulation = true;
    cpu.flags = flags(true, true);
    cpu.flags.zero = false;
    ram.load(0x80FD, &[0xD0, 0x7F, 0xDB]);
    assert_eq!(step(&mut cpu, &mut ram), 4);
}

#[test]
fn jsr_and_rts() {
    let (mut cpu, mut ram) = setup(&[0x20, 0x00, 0x90, 0xDB]);
    ram.load(0x9000, &[0x60]);
    assert_eq!(step(&mut cpu, &mut ram), 6);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(cpu.s, 0x01FD);
    assert_eq!(step(&mut cpu, &mut ram), 6);
    assert_eq!(cpu.pc, 0x8003);
    assert_eq!(cpu.s, 0x01FF);
}

#[test]
fn jmp_indirect() {
    let (mut cpu, mut ram) = setup(&[0x6C, 0x10, 0x00, 0xDB]);
    ram.data[0x10] = 0x00;
    ram.data[0x11] = 0x90;
    assert_eq!(step(&mut cpu, &mut ram), 5);
    assert_eq!(cpu.pc, 0x9000);
}

#[test]
fn jml_indirect_long() {
    let (mut cpu, mut ram) = setup(&[0xDC, 0x10, 0x00, 0xDB]);
    ram.data[0x10] = 0x34;
    ram.data[0x11] = 0x12;
    ram.data[0x12] = 0x02;
    assert_eq!(step(&mut cpu, &mut ram), 6);
    assert_eq!(cpu.pc, 0x1234);
    assert_eq!(cpu.pbr, 0x02);
}

#[test]
fn block_move_copies_and_wraps_counter() {
    let (mut cpu, mut ram) = setup(&[0x54, 0x00, 0x01]);
    cpu.a = 2;
    cpu.x = 0x0010;
    cpu.y = 0x0020;
    ram.data[0x01_0010] = 0xAA;
    ram.data[0x01_0011] = 0xBB;
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x20], 0xAA);
    assert_eq!(cpu.a, 1);
    assert_eq!(cpu.pc, 0x8000);
    step(&mut cpu, &mut ram);
    assert_eq!(ram.data[0x21], 0xBB);
    assert_eq!(cpu.a, 0);
    assert_eq!(cpu.pc, 0x8000);
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0xFFFF);
    assert_eq!(cpu.pc, 0x8003);
}

#[test]
fn wai_wakes_on_masked_irq_without_servicing_it() {
    let (mut cpu, mut ram) = setup(&[0xCB, 0xA9, 0x42, 0xDB]);
    cpu.flags.irq_disable = true;
    step(&mut cpu, &mut ram);
    assert!(cpu.wai);
    cpu.set_irq(true);
    step(&mut cpu, &mut ram);
    assert!(!cpu.wai);
    // The masked IRQ is not serviced; the next instruction is LDA #$42.
    assert_eq!(cpu.pc, 0x8001);
    step(&mut cpu, &mut ram);
    assert_eq!(cpu.a, 0x42);
}

#[test]
fn brk_pushes_return_address_and_uses_irq_vector() {
    let (mut cpu, mut ram) = setup(&[0x00, 0xEA, 0xDB]);
    cpu.emulation = true;
    cpu.flags = flags(true, true);
    ram.data[0xFFFE] = 0x00;
    ram.data[0xFFFF] = 0x90;
    assert_eq!(step(&mut cpu, &mut ram), 7);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(cpu.pbr, 0);
    assert!(cpu.flags.irq_disable);
    assert_eq!(cpu.s, 0x01FC);
    assert_eq!(ram.data[0x01FF], 0x80);
    assert_eq!(ram.data[0x01FE], 0x02);
}

#[test]
fn rti_restores_state() {
    let (mut cpu, mut ram) = setup(&[0x40, 0xDB]);
    cpu.emulation = true;
    cpu.flags = flags(true, true);
    cpu.s = 0x01FB;
    ram.data[0x01FC] = 0x00;
    ram.data[0x01FD] = 0x02;
    ram.data[0x01FE] = 0x80;
    ram.data[0x01FF] = 0x00;
    assert_eq!(step(&mut cpu, &mut ram), 6);
    assert_eq!(cpu.pc, 0x8002);
    assert_eq!(cpu.s, 0x01FE);
}

#[test]
fn wai_services_unmasked_irq() {
    let (mut cpu, mut ram) = setup(&[0xCB, 0xEA, 0xDB]);
    cpu.flags.irq_disable = true;
    ram.data[0xFFEE] = 0x00;
    ram.data[0xFFEF] = 0x90;
    step(&mut cpu, &mut ram);
    assert!(cpu.wai);
    cpu.flags.irq_disable = false;
    cpu.set_irq(true);
    step(&mut cpu, &mut ram); // wake
    assert!(!cpu.wai);
    step(&mut cpu, &mut ram); // service IRQ
    assert_eq!(cpu.pc, 0x9000);
    assert!(cpu.flags.irq_disable);
    assert_eq!(cpu.s, 0x01FB);
}

#[test]
fn jsl_and_rtl() {
    let (mut cpu, mut ram) = setup(&[0x22, 0x00, 0x90, 0x02, 0xDB]);
    ram.load(0x02_9000, &[0x6B]);
    assert_eq!(step(&mut cpu, &mut ram), 8);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(cpu.pbr, 0x02);
    assert_eq!(cpu.s, 0x01FC);
    assert_eq!(step(&mut cpu, &mut ram), 6);
    assert_eq!(cpu.pc, 0x8004);
    assert_eq!(cpu.pbr, 0x00);
    assert_eq!(cpu.s, 0x01FF);
}

#[test]
fn jsr_absolute_indexed_indirect() {
    let (mut cpu, mut ram) = setup(&[0xFC, 0x10, 0x00, 0xDB]);
    cpu.x = 2;
    ram.data[0x12] = 0x00;
    ram.data[0x13] = 0x90;
    ram.load(0x9000, &[0x60]);
    assert_eq!(step(&mut cpu, &mut ram), 8);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(step(&mut cpu, &mut ram), 6);
    assert_eq!(cpu.pc, 0x8003);
}
