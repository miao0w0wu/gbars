use std::path::Path;

use gba_core::gba::GameBoyAdvance;
use gba_core::probe;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rom = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "D:/Games/gba roms/Pokemon.gba".to_string());
    let mut gba = match GameBoyAdvance::load_rom(&rom) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("load failed: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    gba.bus.probe.enabled = std::env::var("DBG_TAIL").is_err();
    let tail_limit: u64 = std::env::var("DBG_TAIL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000_000);
    let mut trail: Vec<u32> = Vec::new();
    let mut last_irq_sp = 0x0300_7FA0u32;
    let mut sp_changes = 0u32;
    let mut prev_now = 0u32;
    for i in 0..30_000_000u64 {
        let t = gba.cpu.t();
        let pc = gba.cpu.reg(15) - if t { 4 } else { 8 };
        trail.push(pc);
        if std::env::var("DBG_DISP").is_ok() && (pc == 0x0800_09DE || pc == 0x0800_09E6) {
            println!(
                "pc={pc:08X} step={i} now={} vcount={} dispstat={:04X} dispcnt={:04X}",
                gba.scheduler.now(),
                gba.bus.ppu().vcount(),
                gba.bus.ppu().dispstat(),
                gba.bus.ppu().dispcnt(),
            );
        }
        if std::env::var("DBG_NOW").is_ok() {
            let now = gba.scheduler.now();
            if i < 40 {
                println!("step={i} pc={pc:08X} now_before={prev_now} now_after={now} t={t}");
            }
            if now.wrapping_sub(prev_now) > 2000 {
                println!("JUMP at step={i} pc={pc:08X} now {prev_now} -> {now}");
            }
            prev_now = now;
        }
        if gba.bus.probe.milestone {
            gba.bus.probe.enabled = false;
            probe::dump_mem(&gba, Path::new("gbars_mem.bin")).expect("dump mem");
            probe::dump_writes(&gba, Path::new("gbars_writes.bin")).expect("dump writes");
            std::fs::write("gbars_pcs.bin", slice_u32(&trail)).expect("dump pcs");
            println!(
                "milestone dump at step {i}: {} write records, pc={pc:#010x}",
                gba.bus.probe.records.len(),
            );
            return std::process::ExitCode::SUCCESS;
        }
        if pc == 0x0300_36A8 {
            static mut D: u32 = 0;
            let n = unsafe { D };
            if n < 12 {
                unsafe {
                    D += 1;
                }
                println!(
                    "Disp #{n} r0={:08X} r12={:08X} IE={:04X} IF={:04X} IME={:04X} dispstat={:04X}",
                    gba.cpu.reg(0),
                    gba.cpu.reg(12),
                    gba.bus.read_u16(0x0400_0200),
                    gba.bus.read_u16(0x0400_0202),
                    gba.bus.read_u16(0x0400_0208),
                    gba.bus.read_u16(0x0400_0004),
                );
            }
        }
        if pc == 0x0000_0134 {
            static mut S: u32 = 0;
            let n = unsafe { S };
            if n < 6 {
                unsafe {
                    S += 1;
                }
                println!(
                    "IrqRet #{n} lr={:08X} sp={:08X} spsr={:08X} cpsr={:08X}",
                    gba.cpu.reg(14),
                    gba.cpu.reg(13),
                    gba.cpu.probe_irq_spsr(),
                    gba.cpu.cpsr(),
                );
            }
        }
        if pc == 0x0000_0134 && gba.cpu.reg(14) < 0x0200_0000 {
            static mut B: u32 = 0;
            let n = unsafe { B };
            if n < 3 {
                unsafe {
                    B += 1;
                }
                println!(
                    "BAD IrqRet #{n} lr={:08X} r0={:08X} r1={:08X} r2={:08X} r3={:08X} r12={:08X} sp={:08X} steps={}",
                    gba.cpu.reg(14),
                    gba.cpu.reg(0),
                    gba.cpu.reg(1),
                    gba.cpu.reg(2),
                    gba.cpu.reg(3),
                    gba.cpu.reg(12),
                    gba.cpu.reg(13),
                    i,
                );
                for a in (0x0300_7F70u32..0x0300_7FA0).step_by(4) {
                    println!("  [{a:08X}] = {:08X}", gba.bus.read_u32(a));
                }
            }
        }
        if pc == 0x0000_0134 {
            static mut R: u32 = 0;
            let n = unsafe { R };
            if n < 10 {
                unsafe {
                    R += 1;
                }
                println!(
                    "IrqRet #{n} pc={pc:08X} lr={:08X} sp={:08X}",
                    gba.cpu.reg(14),
                    gba.cpu.reg(13),
                );
            }
        }
        if pc == 0x0300_3580 || pc == 0x0300_3658 || pc == 0x0300_3694 {
            static mut N: u32 = 0;
            let n = unsafe { N };
            if n < 12 {
                unsafe {
                    N += 1;
                }
                println!(
                    "Intr #{n} pc={pc:08X} r0={:08X} r1={:08X} r2={:08X} r3={:08X} r12={:08X} IE={:04X} IF={:04X} IME={:04X} now={}",
                    gba.cpu.reg(0),
                    gba.cpu.reg(1),
                    gba.cpu.reg(2),
                    gba.cpu.reg(3),
                    gba.cpu.reg(12),
                    gba.bus.read_u16(0x0400_0200),
                    gba.bus.read_u16(0x0400_0202),
                    gba.bus.read_u16(0x0400_0208),
                    gba.scheduler.now(),
                );
            }
        }
        if i + 1 == 30_000_000 {
            gba.bus.probe.enabled = false;
            probe::dump_mem(&gba, Path::new("gbars_mem.bin")).expect("dump mem");
            probe::dump_writes(&gba, Path::new("gbars_writes.bin")).expect("dump writes");
            std::fs::write("gbars_pcs.bin", slice_u32(&trail)).expect("dump pcs");
            println!(
                "dumped at step {}: {} write records, pc={:#010x} IE={:#06x} IME={:#06x} DISPCNT={:#06x} IF={:#06x}",
                i,
                gba.bus.probe.records.len(),
                pc,
                gba.bus.read_u16(0x0400_0200),
                gba.bus.read_u16(0x0400_0208),
                gba.bus.read_u16(0x0400_0000),
                gba.bus.read_u16(0x0400_0202),
            );
            return std::process::ExitCode::SUCCESS;
        }
        if let Ok(lim) = std::env::var("DBG_TAIL") {
            if let Ok(n) = lim.parse::<u64>() {
                if i + 1 == n {
                    println!("--- tail PCs (last 64) ---");
                    for p in trail.iter().rev().take(64).collect::<Vec<_>>().iter().rev() {
                        println!("  {p:08X}");
                    }
                    println!(
                        "stop pc={pc:08X} IE={:04X} IME={:04X} DISPCNT={:04X} IF={:04X} dispstat={:04X}",
                        gba.bus.read_u16(0x0400_0200),
                        gba.bus.read_u16(0x0400_0208),
                        gba.bus.read_u16(0x0400_0000),
                        gba.bus.read_u16(0x0400_0202),
                        gba.bus.read_u16(0x0400_0004),
                    );
                    return std::process::ExitCode::SUCCESS;
                }
            }
        }
        if let Err(e) = gba.scheduler.run_instruction(&mut gba.cpu, &mut gba.bus) {
            eprintln!("error at step {i} pc={pc:#010x}: {e}");
            std::fs::write("gbars_pcs.bin", slice_u32(&trail)).ok();
            return std::process::ExitCode::FAILURE;
        }
        let irq_sp = gba.cpu.probe_irq_sp();
        if irq_sp != last_irq_sp {
            last_irq_sp = irq_sp;
            if irq_sp != 0x0300_7FA0 && irq_sp != 0x0300_7F74 {
                println!(
                    "sp_irq ODD at step {i}: sp={irq_sp:08X} next_pc={:08X}",
                    gba.cpu.reg(15),
                );
            }
        }
    }
    eprintln!("never reached 3M steps");
    std::process::ExitCode::SUCCESS
}

fn slice_u32(v: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}
