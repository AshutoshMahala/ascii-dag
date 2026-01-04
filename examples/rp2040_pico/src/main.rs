//! RP2040 / Raspberry Pi Pico example for ascii-dag
//!
//! This example demonstrates running ascii-dag on embedded hardware,
//! rendering DAGs of various sizes and measuring performance/RAM usage.
//!
//! # Hardware Setup
//! - Connect Pico to USB
//! - Open a serial terminal (e.g., `screen /dev/ttyACM0 115200` or PuTTY on Windows)
//!
//! # Building & Flashing
//! ```bash
//! cargo build --release
//! # Copy target/thumbv6m-none-eabi/release/ascii-dag-pico.uf2 to the Pico
//! # Or use probe-rs:
//! probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/ascii-dag-pico
//! ```

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

// Panic handler
use panic_halt as _;

// RP2040 HAL
use rp2040_hal::{
    clocks::init_clocks_and_plls,
    entry,
    pac,
    timer::Timer,
    usb::UsbBus,
    watchdog::Watchdog,
    Sio,
};

// USB
use usb_device::{class_prelude::*, prelude::*};
use usbd_serial::SerialPort;

// Allocator
use embedded_alloc::Heap;

// ascii-dag
use ascii_dag::DAG;

/// Boot2 bootloader (W25Q080 flash)
#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

/// Heap allocator - 64KB for larger DAGs
#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 64 * 1024; // 64 KB

/// Timing result for a benchmark
struct BenchResult {
    build_us: u64,
    render_us: u64,
    heap_bytes: usize,
    output_bytes: usize,
}

#[entry]
fn main() -> ! {
    // Initialize heap
    {
        use core::mem::MaybeUninit;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }

    // Get peripherals
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    let sio = Sio::new(pac.SIO);

    // Configure clocks (125 MHz)
    let clocks = init_clocks_and_plls(
        12_000_000, // External crystal frequency
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    // Set up timer for benchmarking
    let timer = Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);

    // Set up USB driver
    let usb_bus = UsbBusAllocator::new(UsbBus::new(
        pac.USBCTRL_REGS,
        pac.USBCTRL_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));

    // USB serial port
    let mut serial = SerialPort::new(&usb_bus);

    // USB device descriptor
    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x2E8A, 0x000A))
        .strings(&[StringDescriptors::default()
            .manufacturer("ascii-dag")
            .product("DAG Benchmark")
            .serial_number("001")])
        .unwrap()
        .device_class(usbd_serial::USB_CLASS_CDC)
        .build();

    // Wait for USB enumeration
    let mut enumerated = false;
    while !enumerated {
        if usb_dev.poll(&mut [&mut serial]) {
            enumerated = usb_dev.state() == UsbDeviceState::Configured;
        }
    }

    // Small delay for terminal to connect
    cortex_m::asm::delay(125_000_000); // ~1 second at 125MHz

    // === ASCII-DAG BENCHMARK ===
    
    // Send header
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    write_serial(&mut serial, &mut usb_dev, "╔══════════════════════════════════════════════════╗\r\n");
    write_serial(&mut serial, &mut usb_dev, "║     ascii-dag Performance Benchmark on RP2040    ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "║          Cortex-M0+ @ 125 MHz, 264KB RAM         ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "╚══════════════════════════════════════════════════╝\r\n\r\n");

    let mut buf = String::new();
    
    // ========================================
    // BENCHMARK 1: Small DAG (4 nodes)
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    write_serial(&mut serial, &mut usb_dev, "BENCHMARK 1: Diamond Pattern (4 nodes, 4 edges)\r\n");
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    
    let heap_before = HEAP.used();
    let start = timer.get_counter();
    
    let dag = DAG::from_edges(
        &[(1, "Root"), (2, "Left"), (3, "Right"), (4, "Merge")],
        &[(1, 2), (1, 3), (2, 4), (3, 4)],
    );
    
    let build_time = timer.get_counter() - start;
    let render_start = timer.get_counter();
    
    let output = dag.render();
    
    let render_time = timer.get_counter() - render_start;
    let heap_after = HEAP.used();
    
    write_serial_lines(&mut serial, &mut usb_dev, &output);
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    
    buf.clear();
    write!(buf, "Build:  {:>6} µs\r\n", build_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Render: {:>6} µs\r\n", render_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Heap:   {:>6} bytes (delta: {} bytes)\r\n", heap_after, heap_after - heap_before).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Output: {:>6} bytes\r\n\r\n", output.len()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    
    drop(output);
    drop(dag);

    // ========================================
    // BENCHMARK 2: Medium DAG (10 nodes)
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    write_serial(&mut serial, &mut usb_dev, "BENCHMARK 2: Build Pipeline (10 nodes, 12 edges)\r\n");
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    
    let heap_before = HEAP.used();
    let start = timer.get_counter();
    
    let dag = DAG::from_edges(
        &[
            (1, "Source"),
            (2, "Parse"),
            (3, "Validate"),
            (4, "Transform"),
            (5, "Optimize"),
            (6, "CodeGen"),
            (7, "Link"),
            (8, "Test"),
            (9, "Package"),
            (10, "Deploy"),
        ],
        &[
            (1, 2), (2, 3), (3, 4), (4, 5), (5, 6),
            (6, 7), (7, 8), (8, 9), (9, 10),
            (1, 4), // skip-level: Source -> Transform
            (3, 6), // skip-level: Validate -> CodeGen
            (5, 8), // skip-level: Optimize -> Test
        ],
    );
    
    let build_time = timer.get_counter() - start;
    let render_start = timer.get_counter();
    
    let output = dag.render();
    
    let render_time = timer.get_counter() - render_start;
    let heap_after = HEAP.used();
    
    write_serial_lines(&mut serial, &mut usb_dev, &output);
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    
    buf.clear();
    write!(buf, "Build:  {:>6} µs\r\n", build_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Render: {:>6} µs\r\n", render_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Heap:   {:>6} bytes (delta: {} bytes)\r\n", heap_after, heap_after - heap_before).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Output: {:>6} bytes\r\n\r\n", output.len()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    
    drop(output);
    drop(dag);

    // ========================================
    // BENCHMARK 3: Wide DAG (fan-out/fan-in)
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    write_serial(&mut serial, &mut usb_dev, "BENCHMARK 3: Wide Fan-Out/Fan-In (12 nodes, 16 edges)\r\n");
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    
    let heap_before = HEAP.used();
    let start = timer.get_counter();
    
    let dag = DAG::from_edges(
        &[
            (1, "Start"),
            (2, "Worker1"), (3, "Worker2"), (4, "Worker3"), (5, "Worker4"), (6, "Worker5"),
            (7, "Stage2-A"), (8, "Stage2-B"), (9, "Stage2-C"),
            (10, "Merge1"), (11, "Merge2"),
            (12, "Final"),
        ],
        &[
            // Fan out from Start
            (1, 2), (1, 3), (1, 4), (1, 5), (1, 6),
            // First merge stage
            (2, 7), (3, 7), (4, 8), (5, 9), (6, 9),
            // Second merge
            (7, 10), (8, 10), (8, 11), (9, 11),
            // Final convergence
            (10, 12), (11, 12),
        ],
    );
    
    let build_time = timer.get_counter() - start;
    let render_start = timer.get_counter();
    
    let output = dag.render();
    
    let render_time = timer.get_counter() - render_start;
    let heap_after = HEAP.used();
    
    write_serial_lines(&mut serial, &mut usb_dev, &output);
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    
    buf.clear();
    write!(buf, "Build:  {:>6} µs\r\n", build_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Render: {:>6} µs\r\n", render_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Heap:   {:>6} bytes (delta: {} bytes)\r\n", heap_after, heap_after - heap_before).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Output: {:>6} bytes\r\n\r\n", output.len()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    
    drop(output);
    drop(dag);

    // ========================================
    // BENCHMARK 4: Large Generated DAG
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    write_serial(&mut serial, &mut usb_dev, "BENCHMARK 4: Generated Binary Tree (31 nodes, 30 edges)\r\n");
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    
    // Generate a binary tree: 5 levels = 31 nodes
    let mut nodes: Vec<(usize, &str)> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    
    // Level labels (reused)
    static LABELS: [&str; 31] = [
        "N00", "N01", "N02", "N03", "N04", "N05", "N06", "N07",
        "N08", "N09", "N10", "N11", "N12", "N13", "N14", "N15",
        "N16", "N17", "N18", "N19", "N20", "N21", "N22", "N23",
        "N24", "N25", "N26", "N27", "N28", "N29", "N30",
    ];
    
    for i in 0..31usize {
        nodes.push((i + 1, LABELS[i]));
    }
    
    // Binary tree edges: node i has children 2i and 2i+1
    for i in 1..=15usize {
        edges.push((i, i * 2));
        edges.push((i, i * 2 + 1));
    }
    
    let heap_before = HEAP.used();
    let start = timer.get_counter();
    
    let dag = DAG::from_edges(&nodes, &edges);
    
    let build_time = timer.get_counter() - start;
    let render_start = timer.get_counter();
    
    let output = dag.render();
    
    let render_time = timer.get_counter() - render_start;
    let heap_after = HEAP.used();
    
    // Only show first 20 lines to avoid flooding serial
    write_serial(&mut serial, &mut usb_dev, "(showing first 20 lines)\r\n");
    for (i, line) in output.lines().enumerate() {
        if i >= 20 {
            write_serial(&mut serial, &mut usb_dev, "...(truncated)\r\n");
            break;
        }
        write_serial(&mut serial, &mut usb_dev, line);
        write_serial(&mut serial, &mut usb_dev, "\r\n");
    }
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    
    buf.clear();
    write!(buf, "Build:  {:>6} µs\r\n", build_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Render: {:>6} µs\r\n", render_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Heap:   {:>6} bytes (delta: {} bytes)\r\n", heap_after, heap_after - heap_before).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Output: {:>6} bytes ({} lines)\r\n\r\n", output.len(), output.lines().count()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    
    drop(output);
    drop(dag);
    drop(nodes);
    drop(edges);

    // ========================================
    // BENCHMARK 5: Stress Test - 50 nodes
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    write_serial(&mut serial, &mut usb_dev, "BENCHMARK 5: Stress Test - Linear Chain (50 nodes)\r\n");
    write_serial(&mut serial, &mut usb_dev, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\r\n");
    
    static STRESS_LABELS: [&str; 50] = [
        "S00", "S01", "S02", "S03", "S04", "S05", "S06", "S07", "S08", "S09",
        "S10", "S11", "S12", "S13", "S14", "S15", "S16", "S17", "S18", "S19",
        "S20", "S21", "S22", "S23", "S24", "S25", "S26", "S27", "S28", "S29",
        "S30", "S31", "S32", "S33", "S34", "S35", "S36", "S37", "S38", "S39",
        "S40", "S41", "S42", "S43", "S44", "S45", "S46", "S47", "S48", "S49",
    ];
    
    let mut nodes: Vec<(usize, &str)> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    
    for i in 0..50usize {
        nodes.push((i + 1, STRESS_LABELS[i]));
        if i > 0 {
            edges.push((i, i + 1));
        }
    }
    
    let heap_before = HEAP.used();
    let start = timer.get_counter();
    
    let dag = DAG::from_edges(&nodes, &edges);
    
    let build_time = timer.get_counter() - start;
    let render_start = timer.get_counter();
    
    let output = dag.render();
    
    let render_time = timer.get_counter() - render_start;
    let heap_after = HEAP.used();
    
    // Only show first/last few lines
    write_serial(&mut serial, &mut usb_dev, "(showing first 5 and last 5 lines)\r\n");
    let lines: Vec<&str> = output.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if i < 5 || i >= lines.len() - 5 {
            write_serial(&mut serial, &mut usb_dev, line);
            write_serial(&mut serial, &mut usb_dev, "\r\n");
        } else if i == 5 {
            write_serial(&mut serial, &mut usb_dev, "   ...(middle lines omitted)...\r\n");
        }
    }
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    
    buf.clear();
    write!(buf, "Build:  {:>6} µs\r\n", build_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Render: {:>6} µs\r\n", render_time.ticks()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Heap:   {:>6} bytes (delta: {} bytes)\r\n", heap_after, heap_after - heap_before).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "Output: {:>6} bytes ({} lines)\r\n\r\n", output.len(), lines.len()).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    
    drop(lines);
    drop(output);
    drop(dag);
    drop(nodes);
    drop(edges);

    // ========================================
    // FINAL SUMMARY
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "╔══════════════════════════════════════════════════╗\r\n");
    write_serial(&mut serial, &mut usb_dev, "║                    SUMMARY                       ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "╠══════════════════════════════════════════════════╣\r\n");
    
    let final_heap = HEAP.used();
    let free_heap = HEAP.free();
    
    buf.clear();
    write!(buf, "║  Total heap allocated: {:>6} bytes              ║\r\n", HEAP_SIZE).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "║  Heap currently used:  {:>6} bytes              ║\r\n", final_heap).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "║  Heap free:            {:>6} bytes              ║\r\n", free_heap).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    write_serial(&mut serial, &mut usb_dev, "║                                                  ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "║  ascii-dag runs great on embedded! 🎉           ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "╚══════════════════════════════════════════════════╝\r\n\r\n");
    
    write_serial(&mut serial, &mut usb_dev, "Press any key to run benchmarks again...\r\n");

    // Keep USB alive and wait for keypress to restart
    let mut restart = false;
    loop {
        usb_dev.poll(&mut [&mut serial]);
        
        let mut buf = [0u8; 64];
        if let Ok(count) = serial.read(&mut buf) {
            if count > 0 {
                restart = true;
            }
        }
        
        if restart {
            // Trigger a soft reset
            cortex_m::peripheral::SCB::sys_reset();
        }
        
        cortex_m::asm::wfi(); // Wait for interrupt (low power)
    }
}

/// Write a string to USB serial, handling USB polling
fn write_serial(
    serial: &mut SerialPort<UsbBus>,
    usb_dev: &mut UsbDevice<UsbBus>,
    s: &str,
) {
    let bytes = s.as_bytes();
    let mut written = 0;
    while written < bytes.len() {
        usb_dev.poll(&mut [serial]);
        if let Ok(n) = serial.write(&bytes[written..]) {
            written += n;
        }
    }
    // Flush
    for _ in 0..10 {
        usb_dev.poll(&mut [serial]);
        let _ = serial.flush();
    }
}

/// Write multiline output, converting \n to \r\n for terminal
fn write_serial_lines(
    serial: &mut SerialPort<UsbBus>,
    usb_dev: &mut UsbDevice<UsbBus>,
    s: &str,
) {
    for line in s.lines() {
        write_serial(serial, usb_dev, line);
        write_serial(serial, usb_dev, "\r\n");
    }
}
