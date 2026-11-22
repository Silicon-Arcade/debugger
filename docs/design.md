# Design note: chip6502_debugger

How the debugger is put together, and why it is built the way it is. The
README covers installing and using it; this note is for reading or changing
`src/main.rs`.

## What it is

One egui window around a `Chip6502` and a `ConsoleBus`, shown here with the
built-in program after Reset and one Step, and a breakpoint on `$0203`:

```
+- controls ----------------------------------------------------------------------------------------+
| [Reset] [Step] [Step 16] [Run] | [Load .bin...] [Paste & Save...]                                 |
+- Disassembly ------------------------------------------------------+- Registers ------------------+
| Load at [0200]  Start PC [0200]  Goto [0400] | [^] [v] [|<] [> PC] | A:  $00    (  0)  00000000   |
| ------------------------------------------------------------------ | X:  $00    (  0)  00000000   |
|    $0200  A2 00       0200: LDX #$00 (Immediate)                   | Y:  $00    (  0)  00000000   |
|  > $0202  8A          0202: TXA (Implied)                          | SP: $FD    (stack top $01FD) |
| *  $0203  18          0203: CLC (Implied)                          | PC: $0202                    |
|    $0204  69 30       0204: ADC #$30 (Immediate)                   | P:  $26                      |
|    $0206  20 11 02    0206: JSR $0211 (Absolute)                   | Flags                        |
|    ...                                                             | N V - B D I Z C              |
|                                                                    | 0 0 1 0 0 1 1 0              |
+- Memory -----------------------------------------+- Console output -------------------------------+
| View from [0200]                                 |                                                |
| $0200: A2 00 8A 18 69 30 20 11 02 E8 E0 0A ...   |                                                |
| $0210: 02 8D 01 F0 60 00 00 00 00 00 00 00 ...   |                                                |
| ...                                              |                                                |
+--------------------------------------------------+------------------------------------------------+
```

The diagram is plain ASCII so it lines up in any font. In the window, `>` is
the ▶ PC marker, `*` the ● breakpoint marker, and `[^] [v] [|<]` the ⏶ ⏷ ⏮
page-up, page-down and top-of-listing buttons.

The window opens at 1320 × 700. Each disassembly row is the debugger's own
breakpoint marker, PC marker, address and bytes, followed by the core's
`disassemble_at` text, which begins with its own copy of the address and ends
with the addressing mode.

Everything lives in `src/main.rs`: the `DebuggerApp` struct, its
`eframe::App::update` method, which draws the panels, a handful of free
functions for parsing and formatting, and the tests.

## Where it sits

The debugger is a separate repository with one dependency on the processor:

```
chip6502_debugger  ──depends on──▶  chip6502_core   (github.com/Silicon-Arcade/reference_implementation)
```

The arrow goes one way. The core knows nothing about windows, so it still
builds and tests headless, and anything else built on it (such as book 1's
monitor app) does not pull in egui.

It is a repository of its own, not a crate in the book's workspace, because it
is not one of the builds. A reader of book 1 builds a processor; the debugger
is a tool for looking inside one, and it is useful from the moment Build 3's
programs run. Keeping it outside the build sequence means the chapters do not
have to explain egui panels before they need to, and a reader can clone it at
whatever point it helps.

The dependency is a `path` dependency on `../reference_implementation`, the
published end state of book 1, cloned next to the debugger. A reader with their
own core points the path at their workspace instead; the README lists what the
debugger needs from the crate.

## The seam with the core

The debugger uses only the core's public interface:

| From the core | Where the debugger uses it |
|---|---|
| `Chip6502::new`, `reset`, `step` | construction, Reset, Step, Step 16, Run |
| the `pub` register fields `a x y sp pc p` | the debugger's `registers(&cpu)` snapshot; reading PC; setting PC at start-up, Reset, Load and Start PC |
| `ConsoleBus`, its `ram.memory` and `output_log` | loading bytes, the console panel |
| `Bus::memory_read` | the memory view, the bytes column |
| `disassemble_at(pc, &mut bus)` | the text of each disassembly row |
| `instruction_len_at(opcode)` | the row index, and how many bytes each row shows |

The CPU and the bus are separate values in `DebuggerApp`, and every call that
touches memory takes `&mut self.bus`. That is the core's design (the CPU
borrows the bus for one call at a time rather than owning it), and it means
the debugger can read memory between steps without going through the CPU.

### The register snapshot

`main.rs` defines its own `Registers` struct and a `registers(&cpu)` function
that copies the six fields into it. The copy exists for change highlighting:
before every action that changes the registers (Reset, Step, Step 16, Run,
each auto-step, and committing a new Start PC) the app stores
`prev_regs = registers(&self.cpu)`. The Registers panel compares the live
values against that copy and draws any that differ in amber. `Registers` is
`Copy`, so the snapshot is six bytes' worth of assignment.

Everything else reads the fields in place. Run's halt check, for example,
keeps `self.cpu.pc` from before a step and compares it with the value after.

An earlier draft of this note proposed the opposite arrangement: keep the
core's fields `pub(crate)` and add accessor methods (`registers()`,
`set_pc()`) for tools to call. That protects the core's invariants better, at
the cost of an API the core has to carry for one client. Book 1's core settled
on `pub` fields, so the debugger reads them directly. The cost is on the
debugger's side: it can write `cpu.pc` and nothing stops it writing `cpu.p`
too. It writes only `pc`, in four places: at start-up, after Reset, after a
load, and when Start PC is committed.

## Immediate mode, and what it implies

egui is immediate-mode. There is no tree of widget objects to keep in sync
with the emulator: `update` runs every frame, reads the CPU and bus, and draws
the panels from scratch. A button is an `if ui.button("Step").clicked()` whose
body runs on the frame it was clicked. That suits a debugger, where the
display is a function of machine state and the state changes in jumps.

The price is that whatever `update` draws, it draws every frame. That shapes
the disassembly view.

## The disassembly view

### The row index

A 6502 instruction is one to three bytes, so row *n* of the listing does not
sit at a fixed offset from the start. `compute_disasm_addrs` walks the loaded
region, reading the opcode at each PC and passing it to `instruction_len_at`,
and records each instruction's address in `disasm_addrs: Vec<u16>`. It runs at
start-up, after each load and whenever Start PC is committed
(`recompute_disasm`), never per frame.

The walk is done in two parts: from the load address up to Start PC, then
afresh from Start PC to the end of the loaded bytes. A single walk from the
load address could step over the entry point if the bytes before it decode to
the wrong lengths; starting again at Start PC guarantees a row there. Each
part stops early if the next PC fails to move forward, which guards against
an endless loop at the top of memory.

This is a linear sweep. It cannot tell code from data, so a table in the
middle of a program disassembles as whatever opcodes its bytes make, and the
instructions after it may be misaligned until the walk falls back into step.

### Lazy rendering

With the index built, the listing uses `ScrollArea::show_rows`, which tells
the closure which row range is on screen. Only those rows are formatted and
drawn. Without it, a 64 KB test image produces tens of thousands of label
widgets every frame and the window stops responding.

### Scrolling under program control

Goto, page up and down, ⏮ and **▶ PC** all need to move the listing, but a
`ScrollArea` normally owns its own offset. The app keeps
`disasm_scroll_offset` itself: each frame it passes the value in with
`vertical_scroll_offset`, then reads back where egui actually ended up from
the scroll output. A button changes the stored value; a mouse-wheel scroll
changes egui's, and the read-back keeps it.

Goto and **▶ PC** turn an address into a row with
`disasm_addrs.partition_point(|&pc| pc < target)`, a binary search that works
because the index is in ascending order. If the target lands inside an
instruction, the view goes to the next one. Row index times row height gives
the pixel offset. The page buttons move by 25 rows, which roughly fills the
default window.

### Breakpoints and the current line

Breakpoints are a `HashSet<u16>` of addresses. Each row is a `Label` that
senses clicks; a click toggles that row's address in the set. The row at PC
gets a background color and the ▶ marker.

## Run

Run loops for at most 1,000,000 steps and stops when:

1. PC is on a breakpoint, checked *before* the step, so the breakpointed
   instruction has not run yet. The check is skipped on the first iteration,
   so pressing Run while stopped on a breakpoint moves past it.
2. PC is unchanged after a step: an instruction that jumped to itself, the
   halt convention the book's test programs use.
3. The cap is reached.

The loop runs inside `update`, on the UI thread, so the window does not
repaint until it finishes. The cap bounds how long that can be.

## The other panels

**Address fields.** Load at, Start PC, Goto and View from are all drawn by
`addr_input` over an `AddrInput`, which holds the committed `value` and the
`text` being typed. `parse_addr` accepts surrounding spaces and an optional
`$`. Typing changes nothing; the value is committed when the field loses focus
(Enter or a click elsewhere). Text that does not parse is drawn in the error
color while it is being typed, and committing it puts back the last good
value. `addr_input` returns the new value only on a successful commit, which
is how Start PC knows to move PC and rebuild the listing.

**Memory.** Sixteen rows of sixteen bytes from View from, read through
`Bus::memory_read`, with an ASCII column for `$20`–`$7E`. It reads the bus
every frame, so it shows writes as they happen.

**Console.** `bus.output_log` as lossy UTF-8. `ConsoleBus` appends every
write to `$F001` to that log.

**Paste & Save.** `parse_hex_bytes` scans text for `0x` or `0X` followed by
two hex digits and then a non-hex character (or the end). Everything else is
skipped. That lets it take a Rust `vec![…]` with comments straight from a
listing while ignoring a 16-bit literal such as `0x0200`.

## Known limits

These are in the code as it stands and are left for a later change:

- **A file too large for its load address panics.** `load_program_into_bus`
  asserts the bytes fit in the 64 KB address space; loading 64 KB at `$0200`
  trips it. It should report the problem in the window instead.
- **Auto-step has no switch.** `auto_run` and `last_step` step every 100 ms
  when `auto_run` is true, but no control sets it. A checkbox in the top bar
  would turn it on.
- **Run blocks the window** for as long as it runs (see above).
- **Linear-sweep disassembly** (see above).

## Tests

```sh
cargo test
```

runs three tests that need no window: the built-in program prints
`0123456789` and halts on its `JMP` to itself, the row index puts a row at
each of its instructions, and `disassemble_at` names the first one. Between
them they check that the debugger and the core agree on the API.

## Building a release

```sh
cargo build --release
```

produces `target/release/chip6502_debugger` (`.exe` on Windows), a single
binary. It links only against the C runtime; on Linux the windowing, keyboard
and file-dialog libraries are loaded when it starts, so the target machine
needs a desktop session (X11 or Wayland) with OpenGL and nothing else.

## Possible extensions

- **Watch list:** a panel of chosen addresses, with the previous value shown
  beside any that changed, using the same snapshot-and-compare idea as the
  registers.
- **Symbols:** read a label file from an assembler so the listing shows names.
- **Trace log:** write every executed instruction and the registers to a file,
  for comparing two runs line by line.
- **Machine panels:** a debugger for a machine built on the core would add
  panels for its devices alongside these.

## Appendix: why egui

The choice was made against these alternatives:

| Option | For | Against |
|---|---|---|
| **egui** (chosen) | Pure Rust; Windows, Linux and macOS from one codebase; one binary; calls the emulator directly; immediate mode fits state that changes every step | Layout is less flexible than HTML and CSS; custom styling takes effort; continuous animation needs explicit repaint requests |
| iced | Declarative, Elm-style; cross-platform | Message passing is heavy for a tool that changes a lot of state per frame |
| slint | Declarative markup with a visual designer | A second language to learn, aimed at product UIs |
| Tauri | HTML and CSS front end, Rust back end | Two languages and an IPC boundary between the UI and the emulator |
| Dioxus | React-style; can target web and native | Younger ecosystem |
| ratatui | Terminal UI; works over SSH; no GUI libraries | Less room for a dense register and listing layout |
| WebAssembly in a browser | Shareable by URL | A separate build pipeline, browser file access, harder debugging |
| Local server + web front end | Large UI ecosystem | Two stacks and a WebSocket channel, more than a single-user tool needs |

What decided it: the project is already Rust, it has to run on all three
desktop platforms, and calling `step` and reading registers directly (no IPC)
keeps the gap between pressing Step and seeing the result as small as it can
be.
