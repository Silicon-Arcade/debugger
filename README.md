# chip6502_debugger — a step-through debugger for the chip6502 core

A desktop window for watching a 6502 program run one instruction at a time:
disassembly with the current line highlighted, breakpoints, the six registers
and the status flags (with whatever the last step changed picked out), a
memory view and the console output.

It is a companion to the book *6502 Construction Set*
([see below](#what-is-the-6502-construction-set)). It is not one of the book's
builds; it is a tool to use alongside them. It drives the processor the book builds,
`chip6502_core`, through that crate's public interface only.

Out of the box it builds against the book's [reference
implementation](https://github.com/Silicon-Arcade/reference_implementation),
and you can point it at your own core instead (see [Debugging your own
core](#debugging-your-own-core)).

## What is The 6502 Construction Set?

The 6502 Construction Set is a book that teaches the reader how to build a 6502 emulator
in Rust. It uses 30 core 6502 assembly language patterns as the basis for prioritizing
the implementation of opcodes, and teaches Rust and core systems programming concepts
along the way.

The book includes nostalgia and trivia from the era and finishes by showing
the user how to run Peter Jennings's Microchess on your very own 6502 emulator.

But the emulator is really the vehicle. The goal is to learn how systems software is designed,
tested, and trusted. In a world where AI can generate code in seconds, understanding how
to evaluate that code becomes more important, not less.

- Buy the paperback: https://www.amazon.com/dp/B0HS4N9PR4
- Buy the eBook (DRM free PDF and Ebook): https://leanpub.com/6502constructionset
- Free chapters and the first build - "Hello World!": [silicon-arcade.com](https://silicon-arcade.com/?utm_source=github&utm_medium=readme&utm_campaign=debugger)
- Code: https://github.com/Silicon-Arcade

The Silicon Arcade website also has **episodes**: animated walkthroughs of some of
the book's assembly listings, executing one instruction at a time with the
registers, flags and memory changing as they run —
[silicon-arcade.com/episodes](https://www.silicon-arcade.com/episodes/?utm_source=github&utm_medium=readme&utm_campaign=debugger). I can build
more of these, so if there is a listing you would like to see run, [raise an
issue](../../issues) on this repository and tell me which one.

## What you need

**Rust 1.85 or newer.** The crate declares edition 2024 and a `rust-version`
of 1.85.

```sh
rustup update stable
rustc --version        # expect 1.85.0 or higher
```

**Git**, to clone the two repositories.

**A desktop with OpenGL.** No extra packages are needed to build on Windows,
macOS or Linux. On Linux you need the C toolchain Rust links with (on Debian
or Ubuntu, `sudo apt install build-essential`), and a desktop session (X11 or
Wayland) to run it.

## Install and run

The debugger depends on the processor, `chip6502_core`, from the reference
implementation repository. Cargo finds it by a relative path, so clone both
repositories into the same parent folder:

```sh
git clone https://github.com/Silicon-Arcade/reference_implementation.git
git clone https://github.com/Silicon-Arcade/debugger.git
```

which gives you:

```
reference_implementation/
    chip6502_core/      <- the processor the debugger builds against
    ...
debugger/
    Cargo.toml          <- chip6502_core = { path = "../reference_implementation/chip6502_core" }
    src/main.rs
```

Then build and run the debugger. Cargo builds `chip6502_core` as part of
this; you do not build the reference implementation separately:

```sh
cd debugger
cargo run --release
```

The first build compiles the egui stack and takes a few minutes; later builds
are quick. A window titled **chip6502 debugger** opens with a small built-in
program loaded at `$0200`. Press **Run** and `0123456789` appears in the
console panel.

To check the debugger and the core work together without opening a window,
run its tests:

```sh
cargo test
```

### Debugging your own core

The debugger is written for the book 1 core. To run it against the core you
built while reading book 1, change the path in `debugger/Cargo.toml` to point
at your workspace's `chip6502_core`:

```toml
chip6502_core = { path = "../my-workspace/chip6502_core" }
```

The debugger needs these from the crate. They are what book 1's finished
core provides:

| Item | Used for |
|---|---|
| `Chip6502` with `pub` fields `a`, `x`, `y`, `sp`, `pc`, `p` | the Registers panel; setting PC after a load or reset |
| `Chip6502::new`, `reset(&mut bus)`, `step(&mut bus)` | Reset, Step and Run |
| `ConsoleBus` with `pub ram.memory` and `pub output_log` | loading programs, and the console panel |
| `Bus` trait with `memory_read` | the memory view and the bytes column |
| `disassemble_at(pc, &mut bus)` | the disassembly column |
| `instruction_len_at(opcode)` | walking the program one instruction at a time |

If your names differ, the compiler errors point at the lines to change in
`src/main.rs`.

## Using it

The window has four areas: buttons along the top, the disassembly in the
middle with its address fields above it, registers and flags on the right,
and memory and console output along the bottom.

### The buttons

| Button | What it does |
|---|---|
| **Reset** | Resets the CPU, then sets PC to the **Start PC** address |
| **Step** | Executes one instruction |
| **Step 16** | Executes sixteen |
| **Run** | Executes until PC reaches a breakpoint, an instruction jumps to itself (`JMP *`, the halt convention the book's tests use), or a million instructions have run. The cap stops a runaway loop from freezing the window |
| **Load .bin…** | Loads a raw binary file at the **Load at** address, then sets PC to the **Start PC** address |
| **Paste & Save…** | Opens a window for turning pasted `0xNN` byte literals into a `.bin` file (see below) |

### Address fields

Every address in the window is typed into the same kind of field: four hex
digits, with or without a leading `$` (`0400` and `$0400` both work). Typing
changes nothing until you press Enter or click away. Text that is not a valid
address shows in red, and leaving the field then puts back the last good
value. Hover over a field for a reminder of what it does.

Above the disassembly:

| Field | What it does |
|---|---|
| **Load at** | Where **Load .bin…** puts the file's first byte |
| **Start PC** | Where execution starts, which can differ from where the program is loaded. Setting it moves PC there straight away and rebuilds the disassembly from that address; **Reset** and **Load .bin…** also send PC there |
| **Goto** | Scrolls the listing to an address. It moves the view, not PC |

Above the memory view, **View from** sets the first address shown.

### Loading a program

A `.bin` file here is raw bytes with no header: byte 0 of the file lands at
the load address. Set **Load at** to the load address and **Start PC** to the
address the program starts at, then press **Load .bin…**. For a small program
loaded and started at the same address, set both to the same value. After the
load, the memory view moves to the load address.

For a full 64 KB image, such as one of Klaus Dormann's test binaries, set
**Load at** to `0000` and **Start PC** to the suite's entry point (`0400` for
the functional test) before loading. The suites themselves are GPL-3.0 and
are not in this repository; book 1's Build 26 explains where to get them.

### Disassembly and breakpoints

Each row shows the address, the instruction's bytes and its disassembly. The
row at PC is highlighted and marked ▶. Click any row to set a breakpoint on
it (marked ●); click again to clear it. **Run** stops *before* executing a
breakpointed instruction, and starting **Run** while sitting on a breakpoint
steps past it.

After the address fields:

| Control | What it does |
|---|---|
| ⏶ / ⏷ | Page up and down |
| ⏮ | Back to the top of the loaded program |
| **▶ PC** | Scrolls to the current instruction |

The listing is built by walking the program one instruction at a time: from
the load address up to **Start PC**, then afresh from **Start PC** to the end, so
there is always a row at the entry point. It is rebuilt on each load and
whenever **Start PC** changes. Data mixed in with code disassembles as whatever
opcodes its bytes happen to be.

### Registers and flags

A, X and Y are shown in hex, decimal and binary; SP with the stack address it
points at; then PC and P. The flags row spells out `N V - B D I Z C` with each
bit beneath. Anything the last Step, Step 16, Run or Reset changed is drawn in
amber, so after an `ADC` you see A and the flags it set, and nothing else.

### Memory and console

The memory view shows 256 bytes from **View from** as 16 rows of hex with an ASCII
column. It reads the bus live, so it updates as the program writes.

The console panel shows everything the program has written to the output
port at `$F001`. The built-in program prints `0123456789` there if you press
**Run**.

### Paste & Save

Book listings and test fixtures often hold machine code as Rust byte
literals: `0xA2, 0x00, // LDX #$00`. Paste a block of that into the window
and press **Parse & Save…**. It picks out every `0xNN` with exactly two hex
digits, skips everything else (comments, `vec![`, longer literals such as
`0x0200`), and saves the bytes as a `.bin` you can then load.

## What is here

| Path | What it is |
|---|---|
| `src/main.rs` | The whole debugger: one egui app struct and its panels |
| `docs/design.md` | How it is put together and why: the panels, the lazy disassembly, the choice of egui |
| `LICENSE`, `NOTICE` | The Apache License 2.0, and the notices that go with it |
| `AI_POLICY.md` | The request not to use this repository for AI training |

## License

The code is licensed under the [Apache License 2.0](LICENSE) (see also
[NOTICE](NOTICE)), so you are free to read it, run it, change it, and build on
it, in your own projects or commercial ones. Separately, I ask that this
repository not be used to train, fine-tune, evaluate, or benchmark AI or
machine-learning models without prior written permission; see
[AI_POLICY.md](AI_POLICY.md) for the details.

The book itself, including its text, figures, and listings as published, is
not covered by this license; all rights are reserved.

## Additional Resources

There are many resources describing how the 6502 processor works. A few that I found
particularly helpful were:

- **MOS Technology.** *MCS6500 Microcomputer Family Programming Manual*,
  1976. The reference for instruction behavior, flags and cycle counts.
- **Norbert Landsteiner.** [6502 Instruction Set](https://www.masswerk.at/6502/6502_instruction_set.html),
  masswerk.at. Opcode matrix and instruction details.
- **Andrew Jacobs.** [6502 Reference](https://6502.org/users/obelisk/6502/reference.html),
  6502.org. Per-instruction flag effects and cycle counts.
- **Western Design Center.** *W65C02S datasheet.* Documents the NMOS
  `JMP ($xxFF)` page-wrap bug and the 65C02's differences from the NMOS part.
- **Bruce Clark.** "Decimal Mode," tutorial, [6502.org](https://6502.org/).
  NMOS decimal-mode arithmetic and flags, and the D flag after reset.

The complete list of references can be found in Appendix H of the book.

### Test suites (download them yourself)

- **Klaus Dormann.** [6502/65C02 functional tests](https://github.com/Klaus2m5/6502_65C02_functional_tests).
  GPL-3.0, which is why they are not included here. See
  [Loading a program](#loading-a-program) for how to load one in the debugger.
- **Bruce Clark.** `6502_decimal_test`, distributed with Dormann's tests.
  Public domain.

### Community

- [6502.org](https://6502.org/), maintained by Mike Naberezny. Excellent online community
  containing archives, projects, 6502 source code, tools and forums. The forums are active,
  and the members are knowledgeable and passionate about the chip.

### Learning Rust

- **Steve Klabnik and Carol Nichols.** [*The Rust Programming Language*](https://doc.rust-lang.org/book/)
  ("the Rust book"). Free online.
- [*Rust by Example*](https://doc.rust-lang.org/rust-by-example/). Free online.
- **Jim Blandy, Jason Orendorff and Leonora F. S. Tindall.** *Programming
  Rust*, 2nd edition. O'Reilly, 2021.
- **egui.** [egui.rs](https://www.egui.rs/) and the
  [API documentation](https://docs.rs/egui), for the debugger's user interface.
