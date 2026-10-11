use std::fs;
use std::path::Path;

pub const SYNC_PC: u32 = 0x081E_40A6;

#[derive(Clone, Copy)]
pub struct WriteRec {
    pub pc: u32,
    pub addr: u32,
    pub size: u32,
    pub value: u32,
}

pub struct Probe {
    pub enabled: bool,
    pub dma_active: bool,
    pub hle_active: bool,
    pub pc: u32,
    pub records: Vec<WriteRec>,
    pub milestone: bool,
}

impl Probe {
    pub fn new() -> Self {
        Self {
            enabled: false,
            dma_active: false,
            hle_active: false,
            pc: 0,
            records: Vec::new(),
            milestone: false,
        }
    }

    pub fn log(&mut self, addr: u32, size: u32, value: u32) {
        if !self.enabled || self.dma_active || self.hle_active {
            return;
        }
        let region = addr >> 24;
        if region != 0x03 && region != 0x04 {
            return;
        }
        if self.pc < 0x0200_0000 {
            return;
        }
        self.records.push(WriteRec {
            pc: self.pc,
            addr,
            size,
            value,
        });
        if addr == 0x0300_7E0C && value == 0x4E20 && size == 2 {
            self.milestone = true;
        }
    }
}

pub fn dump_mem(gba: &crate::gba::GameBoyAdvance, path: &Path) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(0x8000 + 0x40000);
    buf.extend_from_slice(gba.bus.probe_iwram());
    buf.extend_from_slice(gba.bus.probe_ewram());
    fs::write(path, buf)
}

pub fn dump_writes(gba: &crate::gba::GameBoyAdvance, path: &Path) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(gba.bus.probe.records.len() * 16);
    for r in &gba.bus.probe.records {
        buf.extend_from_slice(&r.pc.to_le_bytes());
        buf.extend_from_slice(&r.addr.to_le_bytes());
        buf.extend_from_slice(&r.size.to_le_bytes());
        buf.extend_from_slice(&r.value.to_le_bytes());
    }
    fs::write(path, buf)
}
