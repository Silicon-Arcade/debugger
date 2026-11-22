// SPDX-License-Identifier: Apache-2.0 | AI_POLICY.md

use std::collections::HashSet;
use eframe::egui;
use chip6502_core::{Chip6502, ConsoleBus, Bus, disassemble_at, instruction_len_at};

struct DebuggerApp {
    cpu: Chip6502,
    bus: ConsoleBus,
    program_start: u16,
    program_end:   u16,
    breakpoints: HashSet<u16>,   // set of breakpoint addresses
    auto_run: bool,
    last_step: std::time::Instant,
    /// Where `Load .bin…` puts the file's first byte.
    load_at: AddrInput,
    /// Where execution starts, as opposed to where the program was loaded.
    /// Load and Reset set PC here, so a 64 KB image loaded at $0000 can
    /// start at its real entry point, e.g. $0400.
    start_pc: AddrInput,
    /// Address the disassembly scrolls to.
    goto: AddrInput,
    /// First address shown in the memory pane.
    view_from: AddrInput,
    paste_window_open: bool,       // whether the Paste & Save window is shown
    paste_buffer: String,            // multi-line text being pasted/edited
    paste_status: String,            // last parse/save outcome message
    /// Pre-computed PC for each disassembly row in the program_start..program_end
    /// range. Built once when a binary is loaded so the disassembly view can use
    /// `ScrollArea::show_rows` for lazy rendering — without this, large images
    /// (e.g. Klaus's 65 KB interrupt test) hang the UI because every row's
    /// widget would be built every frame.
    disasm_addrs: Vec<u16>,
    /// Current vertical scroll offset of the disassembly view, in pixels.
    /// We track it ourselves so the Goto / Page Up / Page Down buttons can
    /// reposition the view programmatically — we re-pass this to
    /// `ScrollArea::vertical_scroll_offset` every frame and capture the
    /// post-frame value back from the ScrollArea output (so user mouse-wheel
    /// scrolling keeps working).
    disasm_scroll_offset: f32,
    /// Register snapshot taken immediately *before* the most recent action
    /// that executed code (Step, Step 16, Run, auto-run, Reset). The
    /// Registers panel diffs the live registers against this so the user can
    /// see at a glance which registers and flags the last action changed.
    /// `Registers` is `Copy`, so taking a snapshot is a cheap six-field copy.
    prev_regs: Registers,
}

/// A copy of the six programmer-visible registers, taken so the Registers
/// panel can compare the live values with those before the last action.
#[derive(Clone, Copy)]
struct Registers {
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    pc: u16,
    p: u8,
}

/// Snapshot the CPU's registers from its public fields.
fn registers(cpu: &Chip6502) -> Registers {
    Registers { a: cpu.a, x: cpu.x, y: cpu.y, sp: cpu.sp, pc: cpu.pc, p: cpu.p }
}

impl DebuggerApp {
    fn new() -> Self {
        let mut cpu = Chip6502::new();
        let mut bus = ConsoleBus::new();
        cpu.reset(&mut bus);

        // A built-in program, so the window has something to step through
        // before a file is loaded: the "count 0..9" program from Build 3.
        let program: Vec<u8> = vec![
            0xA2, 0x00,             // LDX #$00
            0x8A,                   // TXA
            0x18,                   // CLC
            0x69, 0x30,             // ADC #$30
            0x20, 0x11, 0x02,       // JSR $0211
            0xE8,                   // INX
            0xE0, 0x0A,             // CPX #$0A
            0xD0, 0xF4,             // BNE $0202
            0x4C, 0x0E, 0x02,       // JMP $020E (halt)
            0x8D, 0x01, 0xF0,       // STA $F001
            0x60,                   // RTS
        ];
        let start = 0x0200;

        load_program_into_bus(&mut bus, &program, start);
        cpu.pc = start;

        let program_end = start + program.len() as u16;
        let disasm_addrs = compute_disasm_addrs(&mut bus, start, start, program_end);
        let prev_regs = registers(&cpu);

        Self {
            cpu,
            bus,
            program_start: start,
            program_end,
            breakpoints: HashSet::new(), // initialize empty set of breakpoints
            auto_run: false,
            last_step: std::time::Instant::now(),
            load_at: AddrInput::new(start),
            start_pc: AddrInput::new(start),
            // Pre-fill Goto with `0400` since that's where most "real" code
            // tends to live (Klaus Dormann's tests, for one) — saves a few
            // keystrokes the first time the debugger comes up.
            goto: AddrInput::new(0x0400),
            view_from: AddrInput::new(start),
            paste_window_open: false,
            paste_buffer: String::new(),
            paste_status: String::new(),
            disasm_addrs,
            disasm_scroll_offset: 0.0,
            prev_regs,
        }

    }

    /// Recompute the disassembly row index after a load or a Start PC change.
    /// The result feeds `ScrollArea::show_rows` so the disassembly view
    /// renders lazily. Also resets the scroll offset so the listing opens at
    /// the top (the user can then click `▶ PC` or use Goto to jump to a
    /// specific address).
    fn recompute_disasm(&mut self) {
        self.disasm_addrs = compute_disasm_addrs(
            &mut self.bus,
            self.program_start,
            self.start_pc.value,
            self.program_end,
        );
        self.disasm_scroll_offset = 0.0;
    }
}

/// Build the disassembly row index for the loaded region `start..end`, with
/// a row guaranteed at `entry`. Walking from `start` alone would decode
/// instruction boundaries from the load address, so an entry point that falls
/// mid-instruction (or past data that decodes to the wrong lengths) would get
/// no row. Instead the region before `entry` is walked on its own, and a
/// fresh walk starts exactly at `entry`. Rows stay ascending, which the Goto
/// and `▶ PC` searches rely on.
fn compute_disasm_addrs(bus: &mut ConsoleBus, start: u16, entry: u16, end: u16) -> Vec<u16> {
    let entry = entry.clamp(start, end);
    let mut addrs = walk_instructions(bus, start, entry);
    addrs.extend(walk_instructions(bus, entry, end));
    addrs
}

/// Walk `pc` from `start` while it is below `end`, one instruction at a
/// time, returning the PC of each instruction. Stops if the PC ever fails to
/// advance, which would otherwise loop forever (e.g. an opcode whose
/// addressing-mode length is 0, or wrap at $FFFF).
fn walk_instructions(bus: &mut ConsoleBus, start: u16, end: u16) -> Vec<u16> {
    let mut addrs = Vec::new();
    let mut pc = start;
    while pc < end {
        addrs.push(pc);
        let n_bytes = u16::from(instruction_len_at(bus.memory_read(pc)));
        let next = pc.wrapping_add(n_bytes);
        if next <= pc {
            break; // wrap-around or zero-length step
        }
        pc = next;
    }
    addrs
}

impl eframe::App for DebuggerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Top: control buttons. Address inputs live above the
        // disassembly, so this bar holds only actions. ---
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Reset the CPU, then point the PC at the Start PC address. The
                // real reset sequence reads the PC from the vector at
                // $FFFC/$FFFD, but nothing in this debugger writes that
                // vector, so on a zeroed bus it would send the PC to $0000 —
                // off the top of the disassembly and into a sea of BRKs.
                // This mirrors what `Load .bin…` already does.
                if ui.button("Reset").clicked() {
                    self.prev_regs = registers(&self.cpu);
                    self.cpu.reset(&mut self.bus);
                    self.cpu.pc = self.start_pc.value;
                }
                if ui.button("Step").clicked() {
                    self.prev_regs = registers(&self.cpu);
                    self.cpu.step(&mut self.bus);
                }
                if ui.button("Step 16").clicked() {
                    self.prev_regs = registers(&self.cpu);
                    for _ in 0..16 {
                        self.cpu.step(&mut self.bus);
                    }
                }

                // Auto-step at fixed rate
                if self.auto_run && self.last_step.elapsed() >= std::time::Duration::from_millis(100) {
                    self.prev_regs = registers(&self.cpu);
                    self.cpu.step(&mut self.bus);
                    self.last_step = std::time::Instant::now();
                    ctx.request_repaint();   // keep the UI updating
                }

                // Run until halt or breakpoint
                if ui.button("Run").clicked() {
                    self.prev_regs = registers(&self.cpu);
                    let max_steps = 1_000_000;     // safety cap
                    let mut first = true;
                    for _ in 0..max_steps {
                        let pc_now = self.cpu.pc;
                        if !first && self.breakpoints.contains(&pc_now) {
                            break;                   // stop BEFORE executing breakpoint
                        }
                        first = false;
                        self.cpu.step(&mut self.bus);
                        if self.cpu.pc == pc_now {
                            break;                   // JMP-self halt convention
                        }
                    }
                }

                ui.separator();

                // Load a program from a .bin file.
                //
                // The load address is taken from the Load at field. Set it
                // to $0000 before loading a full-64 KB image (e.g. Klaus
                // Dormann's `6502_functional_test.bin`); set it to $0200 (or
                // wherever) for a small relocatable program. PC is set to
                // the Start PC field, which can differ from Load at.
                if ui.button("Load .bin…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("6502 binary", &["bin"])
                        .add_filter("All files",   &["*"])
                        .pick_file()
                    {
                        match std::fs::read(&path) {
                            Ok(bytes) => {
                                let load_at = self.load_at.value;
                                self.cpu.reset(&mut self.bus);
                                load_program_into_bus(&mut self.bus, &bytes, load_at);
                                self.cpu.pc = self.start_pc.value;
                                self.view_from.set(load_at);
                                self.program_start = load_at;
                                // saturating_add handles a full-64 KB image
                                // loaded at $0000 (where load_at + 65536 would
                                // otherwise overflow u16).
                                self.program_end = load_at.saturating_add(
                                    bytes.len().min(0xFFFF) as u16
                                );
                                // Rebuild the disassembly row index so the
                                // central panel can render the new program
                                // lazily — without this, a 65 KB binary would
                                // hang the UI building tens of thousands of
                                // widgets per frame.
                                self.recompute_disasm();
                            }
                            Err(e) => eprintln!("Failed to read {:?}: {}", path, e),
                        }
                    }
                }

                if ui.button("Paste & Save…").clicked() {
                    self.paste_window_open = true;
                }
            });
        });

        // --- Right: register / flag display ---
        egui::SidePanel::right("registers")
            .resizable(false)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.heading("Registers");
                let r = registers(&self.cpu);
                let prev = self.prev_regs;

                // Each row is highlighted when its value differs from the
                // snapshot taken before the last action, so stepping through
                // `LDX #$00` lights up X, `ADC` lights up A and the flags, etc.
                register_row(ui, format!("A:  ${:02X}    ({:>3})  {:08b}", r.a, r.a, r.a), r.a != prev.a);
                register_row(ui, format!("X:  ${:02X}    ({:>3})  {:08b}", r.x, r.x, r.x), r.x != prev.x);
                register_row(ui, format!("Y:  ${:02X}    ({:>3})  {:08b}", r.y, r.y, r.y), r.y != prev.y);
                register_row(ui, format!("SP: ${:02X}    (stack top $01{:02X})", r.sp, r.sp), r.sp != prev.sp);
                register_row(ui, format!("PC: ${:04X}", r.pc), r.pc != prev.pc);
                register_row(ui, format!("P:  ${:02X}", r.p), r.p != prev.p);

                ui.separator();
                ui.heading("Flags");
                flags_row(ui, r.p, prev.p);
                ui.monospace(format_flags(r.p));

                ui.separator();
                ui.small(
                    egui::RichText::new("amber = changed by the last Step / Run / Reset")
                        .color(CHANGED_COLOR),
                );
            });

        egui::TopBottomPanel::bottom("bottom").resizable(true)
            .default_height(180.0)
            .show(ctx, |ui| {
                ui.columns(2, |cols| {
                    // --- Left column: memory view ---
                    cols[0].heading("Memory");
                    cols[0].horizontal(|ui| {
                        addr_input(
                            ui,
                            "View from",
                            "First address shown below",
                            &mut self.view_from,
                        );
                    });
                    egui::ScrollArea::vertical()
                        .id_source("mem_scroll")
                        .show(&mut cols[0], |ui| {
                            let start = self.view_from.value;
                            for row in 0..16u16 {
                                let row_addr = start.wrapping_add(row * 16);
                                let mut hex = String::new();
                                let mut ascii = String::new();
                                for col in 0..16u16 {
                                    let b = self.bus.memory_read(row_addr.wrapping_add(col));
                                    hex.push_str(&format!("{:02X} ", b));
                                    ascii.push(if (0x20..=0x7E).contains(&b) {
                                        b as char
                                    } else {
                                        '.'
                                    });
                                }
                                ui.monospace(format!("${:04X}: {} {}", row_addr, hex, ascii));
                            }
                        });

                    // --- Right column: console output ---
                    cols[1].heading("Console output");
                    egui::ScrollArea::vertical()
                        .id_source("console_scroll")
                        .show(&mut cols[1], |ui| {
                            let log = &self.bus.output_log;
                            let text = String::from_utf8_lossy(log);
                            ui.monospace(text);
                        });
                });
            });

        // --- Center: disassembly view (lazy-rendered) ---
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Disassembly");

            let total_rows = self.disasm_addrs.len();

            // Row height (with spacing) — used both by ScrollArea::show_rows
            // for virtualisation and by the Goto / Page-Up / Page-Down logic
            // below to convert a row index into a pixel offset.
            let row_height = ui
                .text_style_height(&egui::TextStyle::Monospace)
                + ui.spacing().item_spacing.y;

            // --- Address bar: the three addresses that drive loading,
            // execution and the view, then the paging buttons -----------
            // `disasm_addrs` is built ascending, so `partition_point` is a
            // fast binary-search for the first row whose PC ≥ target.
            ui.horizontal(|ui| {
                addr_input(
                    ui,
                    "Load at",
                    "Where Load .bin… puts the file's first byte",
                    &mut self.load_at,
                );

                // Committing Start PC moves PC there at once and rebuilds the
                // listing so it has a row at the new address.
                if addr_input(
                    ui,
                    "Start PC",
                    "Where execution starts. Sets PC now, and on Reset and Load",
                    &mut self.start_pc,
                )
                .is_some()
                {
                    self.prev_regs = registers(&self.cpu);
                    self.cpu.pc = self.start_pc.value;
                    self.recompute_disasm();
                }

                if let Some(target) = addr_input(
                    ui,
                    "Goto",
                    "Scroll the listing to this address. Does not change PC",
                    &mut self.goto,
                ) {
                    if total_rows > 0 {
                        let row = self
                            .disasm_addrs
                            .partition_point(|&pc| pc < target)
                            .min(total_rows - 1);
                        self.disasm_scroll_offset = row as f32 * row_height;
                    }
                }

                ui.separator();

                // Page rows ~= one screenful. 25 is roughly right for the
                // default window height; the user can also wheel-scroll.
                let page_rows = 25.0;
                if ui.button("⏶").on_hover_text("Page up").clicked() {
                    self.disasm_scroll_offset =
                        (self.disasm_scroll_offset - page_rows * row_height).max(0.0);
                }
                if ui.button("⏷").on_hover_text("Page down").clicked() {
                    let max_offset = (total_rows as f32 * row_height
                        - row_height * page_rows)
                        .max(0.0);
                    self.disasm_scroll_offset =
                        (self.disasm_scroll_offset + page_rows * row_height).min(max_offset);
                }
                if ui.button("⏮").on_hover_text("Top of the listing").clicked() {
                    self.disasm_scroll_offset = 0.0;
                }
                if ui.button("▶ PC").on_hover_text("Scroll to current PC").clicked()
                    && total_rows > 0
                {
                    let pc = self.cpu.pc;
                    let row = self
                        .disasm_addrs
                        .partition_point(|&p| p < pc)
                        .min(total_rows - 1);
                    self.disasm_scroll_offset = row as f32 * row_height;
                }
            });
            ui.separator();

            if total_rows == 0 {
                ui.label("(no program loaded)");
                return;
            }

            let current_pc = self.cpu.pc;

            // Pass our tracked offset to the ScrollArea each frame, then
            // capture whatever offset egui ends up at (in case the user
            // mouse-wheel-scrolled inside the area). This keeps user scroll
            // working *and* lets the buttons above programmatically jump.
            let scroll_output = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .vertical_scroll_offset(self.disasm_scroll_offset)
                .show_rows(ui, row_height, total_rows, |ui, row_range| {
                    for row in row_range {
                        let pc = self.disasm_addrs[row];
                        let line = disassemble_at(pc, &mut self.bus);
                        let n_bytes = u16::from(instruction_len_at(self.bus.memory_read(pc)));

                        // Format the bytes column, e.g. "A2 00" or "20 11 02"
                        let bytes: String = (0..n_bytes)
                            .map(|i| format!("{:02X}", self.bus.memory_read(pc.wrapping_add(i))))
                            .collect::<Vec<_>>()
                            .join(" ");

                        let bp_marker = if self.breakpoints.contains(&pc) { "●" } else { " " };
                        let row_text = format!(
                            "{}{} ${:04X}  {:<10}  {}",
                            bp_marker,
                            if pc == current_pc { "▶" } else { " " },
                            pc,
                            bytes,
                            line,
                        );

                        let mut text = egui::RichText::new(row_text).monospace();
                        if pc == current_pc {
                            text = text.background_color(egui::Color32::from_rgb(40, 80, 120));
                        }

                        let response = ui.add(
                            egui::Label::new(text).sense(egui::Sense::click())
                        );
                        if response.clicked() {
                            if self.breakpoints.contains(&pc) {
                                self.breakpoints.remove(&pc);
                            } else {
                                self.breakpoints.insert(pc);
                            }
                        }
                    }
                });
            // Capture the scroll offset egui ended up at so the next frame's
            // `vertical_scroll_offset(self.disasm_scroll_offset)` mirrors it,
            // letting user-initiated scrolling (mouse wheel, scrollbar drag)
            // persist instead of being reset.
            self.disasm_scroll_offset = scroll_output.state.offset.y;
        });

        // --- Paste & Save floating window ---
        let mut open = self.paste_window_open;
        egui::Window::new("Paste machine code")
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.label(
                    "Paste hex bytes (e.g. 0xA9, 0x39, ...). Comments and surrounding code are ignored. \
                     Only exactly two-digit literals like 0xNN are matched."
                );

                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.paste_buffer)
                                .font(egui::TextStyle::Monospace)
                                .desired_rows(15)
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.horizontal(|ui| {
                    if ui.button("Parse & Save…").clicked() {
                        match parse_hex_bytes(&self.paste_buffer) {
                            Ok(bytes) => {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("6502 binary", &["bin"])
                                    .save_file()
                                {
                                    match std::fs::write(&path, &bytes) {
                                        Ok(()) => {
                                            self.paste_status = format!(
                                                "Saved {} bytes to {}",
                                                bytes.len(),
                                                path.display()
                                            );
                                        }
                                        Err(e) => {
                                            self.paste_status = format!("Save failed: {}", e);
                                        }
                                    }
                                } else {
                                    self.paste_status = format!(
                                        "Parsed {} bytes (save canceled)",
                                        bytes.len()
                                    );
                                }
                            }
                            Err(e) => {
                                self.paste_status = format!("Parse error: {}", e);
                            }
                        }
                    }
                    if ui.button("Clear").clicked() {
                        self.paste_buffer.clear();
                        self.paste_status.clear();
                    }
                });

                if !self.paste_status.is_empty() {
                    ui.label(&self.paste_status);
                }
            });
        self.paste_window_open = open;
    }
}

/// Scan `text` for `0xNN` byte literals and return them as a `Vec<u8>`.
///
/// Matches exactly two hex digits with a non-hex-digit boundary after, so
/// `0x0200` (an address) is skipped rather than mis-parsed. Comments and
/// surrounding code are ignored because non-matching bytes are simply
/// stepped over.
fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, String> {
    let bytes_in = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes_in.len() {
        let is_byte_lit = i + 4 <= bytes_in.len()
            && bytes_in[i] == b'0'
            && (bytes_in[i + 1] == b'x' || bytes_in[i + 1] == b'X')
            && bytes_in[i + 2].is_ascii_hexdigit()
            && bytes_in[i + 3].is_ascii_hexdigit()
            && (i + 4 == bytes_in.len() || !bytes_in[i + 4].is_ascii_hexdigit());
        if is_byte_lit {
            let hex = std::str::from_utf8(&bytes_in[i + 2..i + 4]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 4;
        } else {
            i += 1;
        }
    }
    if out.is_empty() {
        Err("no 0xNN byte literals found".to_string())
    } else {
        Ok(out)
    }
}

/// Color used throughout the Registers panel to mark a value that changed
/// on the most recent action.
const CHANGED_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 60);

/// Status-flag letters in bit order 7..=0. Bit 5 has no meaning on the 6502
/// and is shown as `-`.
const FLAG_NAMES: [char; 8] = ['N', 'V', '-', 'B', 'D', 'I', 'Z', 'C'];

/// One monospace line in the Registers panel. When `changed` is true the
/// text is drawn bold in the highlight color so the eye is drawn to it.
fn register_row(ui: &mut egui::Ui, text: String, changed: bool) {
    let mut rich = egui::RichText::new(text).monospace();
    if changed {
        rich = rich.color(CHANGED_COLOR).strong();
    }
    ui.label(rich);
}

/// Render the status register as a row of flag letters. A set flag is drawn
/// bright and bold, a clear flag dim; a flag that differs between `p` and
/// `prev_p` is additionally drawn on a highlighted background so a single
/// step's effect on the flags is obvious.
fn flags_row(ui: &mut egui::Ui, p: u8, prev_p: u8) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, name) in FLAG_NAMES.iter().enumerate() {
            // FLAG_NAMES is indexed 0..8 with `N` first, so the mask for
            // index `i` is bit `7 - i`.
            let mask: u8 = 1 << (7 - i);
            let is_set = p & mask != 0;
            let changed = (p ^ prev_p) & mask != 0;

            let mut rich = egui::RichText::new(format!("{name} ")).monospace();
            rich = if is_set { rich.strong() } else { rich.weak() };
            if changed {
                rich = rich.color(CHANGED_COLOR).strong();
            }
            ui.label(rich);
        }
    });
}

/// Render the 8 status-flag bits as "1 0 1 0 0 0 1 0" with spaces.
fn format_flags(p: u8) -> String {
    (0..8).rev()
        .map(|bit| if p & (1 << bit) != 0 { '1' } else { '0' })
        .fold(String::new(), |mut acc, c| {
            if !acc.is_empty() { acc.push(' '); }
            acc.push(c);
            acc
        })
}

fn main() -> Result<(), eframe::Error> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 700.0])
            .with_title("chip6502 debugger"),
        ..Default::default()
    };
    eframe::run_native(
        "chip6502 debugger",
        opts,
        Box::new(|_cc| Ok(Box::new(DebuggerApp::new()))),
    )
}

fn load_program_into_bus(bus: &mut ConsoleBus, bytes: &[u8], at: u16) {
    let start = at as usize;
    let end = start + bytes.len();
    assert!(
        end <= bus.ram.memory.len(),
        "program too large: {} bytes at ${at:04X}",
        bytes.len()
    );
    bus.ram.memory[start..end].copy_from_slice(bytes);
}

/// A hex address the user can edit: the committed `value` and the `text`
/// being typed, which may be half-finished or invalid.
struct AddrInput {
    value: u16,
    text: String,
}

impl AddrInput {
    fn new(value: u16) -> Self {
        Self { value, text: format!("{value:04X}") }
    }

    fn set(&mut self, value: u16) {
        *self = Self::new(value);
    }
}

/// Parse a hex address, ignoring surrounding whitespace and an optional `$`
/// prefix, so "000A", "$000A" and "  0a  " all give $000A.
fn parse_addr(text: &str) -> Option<u16> {
    u16::from_str_radix(text.trim().trim_start_matches('$'), 16).ok()
}

/// A labeled hex address field. Every address input in the window uses it,
/// so they all look and behave the same: typing changes nothing until the
/// user presses Enter or leaves the field, which commits the value and
/// returns it. Text that is not a valid address is drawn in the error
/// color, and committing it puts back the last good value.
fn addr_input(ui: &mut egui::Ui, label: &str, hint: &str, input: &mut AddrInput) -> Option<u16> {
    ui.label(label);
    let valid = parse_addr(&input.text).is_some();
    let mut edit = egui::TextEdit::singleline(&mut input.text)
        .font(egui::TextStyle::Monospace)
        .desired_width(40.0);
    if !valid {
        edit = edit.text_color(ui.visuals().error_fg_color);
    }
    let response = ui.add(edit).on_hover_text(hint);

    if !response.lost_focus() {
        return None;
    }
    match parse_addr(&input.text) {
        Some(value) => {
            input.set(value);
            Some(value)
        }
        None => {
            input.set(input.value);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_program_prints_digits_and_halts() {
        let mut app = DebuggerApp::new();
        for _ in 0..1_000 {
            let pc_before = app.cpu.pc;
            app.cpu.step(&mut app.bus);
            if app.cpu.pc == pc_before {
                break;
            }
        }
        assert_eq!(app.bus.output_log, b"0123456789");
        assert_eq!(app.cpu.pc, 0x020E, "halted on the JMP-self");
    }

    #[test]
    fn disassembly_rows_start_at_each_instruction() {
        let app = DebuggerApp::new();
        assert_eq!(
            app.disasm_addrs,
            [0x0200, 0x0202, 0x0203, 0x0204, 0x0206, 0x0209, 0x020A, 0x020C, 0x020E, 0x0211, 0x0214]
        );
    }

    #[test]
    fn disassembly_line_names_the_instruction() {
        let mut app = DebuggerApp::new();
        assert!(disassemble_at(0x0200, &mut app.bus).contains("LDX #$00"));
    }
}
