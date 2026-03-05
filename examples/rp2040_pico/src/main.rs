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
// ascii-dag
use ascii_dag::Graph;
use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::csr::CsrGraphBuilder;

/// Boot2 bootloader (W25Q080 flash)
#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

/// Heap allocator - 80KB for Heap tests
#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 80 * 1024; // 80 KB

/// Arena buffer - 80KB for Arena tests (statically allocated)
/// We use 'static mut' which is unsafe but standard for embedded single-core No-OS tests
static mut ARENA_BUF: [u8; 80 * 1024] = [0u8; 80 * 1024];

/// Timing result for a benchmark (3-phase timing)
#[derive(Clone, Copy)]
struct BenchResult {
    build_us: u64,   // Graph construction
    compute_us: u64, // Layout computation
    render_us: u64,  // Rendering to string
    memory_bytes: usize,
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
    write_serial(&mut serial, &mut usb_dev, "╔═══════════════════════════════════════════════════════════════════╗\r\n");
    write_serial(&mut serial, &mut usb_dev, "║         ascii-dag Performance Benchmark on RP2040 Pico           ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "║            Comparing: Heap (std) vs Arena (no_std)               ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "╚═══════════════════════════════════════════════════════════════════╝\r\n\r\n");

    let mut buf = String::new();

    // === TEST 1: 10 Node Chain ===
    let mut nodes_10: Vec<(usize, String)> = Vec::new();
    let mut edges_10: Vec<(usize, usize)> = Vec::new();
    for i in 0..10 {
        nodes_10.push((i + 1, alloc::format!("N{:02}", i)));
        if i > 0 { edges_10.push((i, i + 1)); }
    }
    let node_refs_10: Vec<(usize, &str)> = nodes_10.iter().map(|(id, s)| (*id, s.as_str())).collect();
    run_comparison("Chain 10", &node_refs_10, &edges_10, &timer, &mut serial, &mut usb_dev, &mut buf);

    // === TEST 2: 50 Node Chain ===
    let mut nodes_50: Vec<(usize, String)> = Vec::new();
    let mut edges_50: Vec<(usize, usize)> = Vec::new();
    for i in 0..50 {
        nodes_50.push((i + 1, alloc::format!("N{:02}", i)));
        if i > 0 { edges_50.push((i, i + 1)); }
    }
    let node_refs_50: Vec<(usize, &str)> = nodes_50.iter().map(|(id, s)| (*id, s.as_str())).collect();
    run_comparison("Chain 50", &node_refs_50, &edges_50, &timer, &mut serial, &mut usb_dev, &mut buf);

    // === TEST 3: 100 Node Chain ===
    let mut nodes_100: Vec<(usize, String)> = Vec::new();
    let mut edges_100: Vec<(usize, usize)> = Vec::new();
    for i in 0..100 {
        nodes_100.push((i + 1, alloc::format!("N{:02}", i)));
        if i > 0 { edges_100.push((i, i + 1)); }
    }
    let node_refs_100: Vec<(usize, &str)> = nodes_100.iter().map(|(id, s)| (*id, s.as_str())).collect();
    run_comparison("Chain 100", &node_refs_100, &edges_100, &timer, &mut serial, &mut usb_dev, &mut buf);

    // ========================================
    // FINAL SUMMARY
    // ========================================
    write_serial(&mut serial, &mut usb_dev, "\r\n");
    write_serial(&mut serial, &mut usb_dev, "╔═══════════════════════════════════════════════════════════════════╗\r\n");
    write_serial(&mut serial, &mut usb_dev, "║                          SUMMARY                                 ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "╠═══════════════════════════════════════════════════════════════════╣\r\n");
    
    let final_heap = HEAP.used();
    let free_heap = HEAP.free();
    
    buf.clear();
    write!(buf, "║  Heap Pool:     {:>6} bytes                                      ║\r\n", HEAP_SIZE).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "║  Heap Used:     {:>6} bytes                                      ║\r\n", final_heap).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    buf.clear();
    write!(buf, "║  Heap Free:     {:>6} bytes                                      ║\r\n", free_heap).ok();
    write_serial(&mut serial, &mut usb_dev, &buf);
    write_serial(&mut serial, &mut usb_dev, "║                                                                   ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "║  Arena mode uses ZERO heap - pure stack/static allocation!       ║\r\n");
    write_serial(&mut serial, &mut usb_dev, "╚═══════════════════════════════════════════════════════════════════╝\r\n\r\n");
    
    write_serial(&mut serial, &mut usb_dev, "Press any key to run benchmarks again...\r\n");
    
    // Extra flush - poll USB many times to ensure all data is transmitted
    for _ in 0..100 {
        usb_dev.poll(&mut [&mut serial]);
        let _ = serial.flush();
        cortex_m::asm::delay(10_000); // Small delay between polls
    }

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
        // Small delay to let USB process
        cortex_m::asm::delay(1000);
    }
    // Flush more aggressively
    for _ in 0..20 {
        usb_dev.poll(&mut [serial]);
        let _ = serial.flush();
        cortex_m::asm::delay(1000);
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

/// Run comparison for a specific graph scenario
fn run_comparison(
    name: &str,
    nodes: &[(usize, &str)],
    edges: &[(usize, usize)],
    timer: &Timer,
    serial: &mut SerialPort<UsbBus>,
    usb_dev: &mut UsbDevice<UsbBus>,
    buf: &mut String,
) {
    write_serial(serial, usb_dev, "═══════════════════════════════════════════════════════════════════════\r\n");
    buf.clear();
    write!(buf, " {} ({} nodes, {} edges)\r\n", name, nodes.len(), edges.len()).ok();
    write_serial(serial, usb_dev, buf);
    write_serial(serial, usb_dev, "───────────────────────────────────────────────────────────────────────\r\n");

    // --- Heap Run ---
    let heap_stats = run_heap_benchmark(nodes, edges, timer);
    
    // --- Arena Run ---
    let arena_stats = run_arena_benchmark(nodes, edges, timer, serial, usb_dev);
    
    // --- Report with 3-phase timing ---
    write_serial(serial, usb_dev, "Mode   | Build (us) | Compute (us) | Render (us) | Total (us) | Memory\r\n");
    write_serial(serial, usb_dev, "-------|------------|--------------|-------------|------------|--------\r\n");
    
    buf.clear();
    let heap_total = heap_stats.build_us + heap_stats.compute_us + heap_stats.render_us;
    write!(buf, "HEAP   | {:>10} | {:>12} | {:>11} | {:>10} | {:>6}\r\n", 
           heap_stats.build_us, heap_stats.compute_us, heap_stats.render_us,
           heap_total, heap_stats.memory_bytes).ok();
    write_serial(serial, usb_dev, buf);

    buf.clear();
    if let Some(stats) = arena_stats {
        let arena_total = stats.build_us + stats.compute_us + stats.render_us;
        write!(buf, "ARENA  | {:>10} | {:>12} | {:>11} | {:>10} | {:>6}\r\n", 
            stats.build_us, stats.compute_us, stats.render_us,
            arena_total, stats.memory_bytes).ok();
    } else {
        write!(buf, "ARENA  |     FAILED |       FAILED |      FAILED |     FAILED |    OOM\r\n").ok();
    }
    write_serial(serial, usb_dev, buf);
    
    // Show speedup if arena succeeded
    if let Some(stats) = arena_stats {
        let heap_total = heap_stats.build_us + heap_stats.compute_us + heap_stats.render_us;
        let arena_total = stats.build_us + stats.compute_us + stats.render_us;
        if arena_total > 0 {
            let speedup = (heap_total as f32) / (arena_total as f32);
            buf.clear();
            write!(buf, "                                          Speedup: {:.2}x\r\n", speedup).ok();
            write_serial(serial, usb_dev, buf);
        }
    }
    write_serial(serial, usb_dev, "\r\n");
}

fn run_heap_benchmark(nodes: &[(usize, &str)], edges: &[(usize, usize)], timer: &Timer) -> BenchResult {
    let heap_before = HEAP.used();
    
    // Phase 1: Build
    let build_start = timer.get_counter();
    let dag = Graph::from_edges(nodes, edges);
    let build_time = timer.get_counter() - build_start;
    
    // Phase 2: Compute layout
    let compute_start = timer.get_counter();
    let ir = dag.compute_layout();
    let compute_time = timer.get_counter() - compute_start;
    
    // Phase 3: Render
    let render_start = timer.get_counter();
    let output = ir.render_scanline();
    let render_time = timer.get_counter() - render_start;
    
    let heap_after = HEAP.used();
    
    drop(output);
    drop(dag);
    
    BenchResult {
        build_us: build_time.ticks(),
        compute_us: compute_time.ticks(),
        render_us: render_time.ticks(),
        memory_bytes: heap_after.saturating_sub(heap_before),
    }
}

fn run_arena_benchmark(
    nodes: &[(usize, &str)], 
    edges: &[(usize, usize)], 
    timer: &Timer,
    // Debug output
    serial: &mut SerialPort<UsbBus>,
    usb_dev: &mut UsbDevice<UsbBus>,
) -> Option<BenchResult> {
    unsafe {
        // Memory layout: 80KB total
        // Graph: 10KB (structural data)
        // Temp: 35KB (layout calculation scratch)
        // Output: 35KB (final layout result)
        let (graph_mem, rest) = ARENA_BUF.split_at_mut(10 * 1024);
        let (layout_temp_mem, output_mem) = rest.split_at_mut(35 * 1024);
        
        // Phase 1: Build graph
        let build_start = timer.get_counter();
        
        let mut graph_arena = Arena::new(graph_mem);
        let label_bytes = nodes.iter().map(|(_, l)| l.len()).sum::<usize>() + 256;
        let mut builder = match CsrGraphBuilder::new(&mut graph_arena, nodes.len(), edges.len(), label_bytes) {
            Some(b) => b,
            None => {
                write_serial(serial, usb_dev, "  [Arena FAIL: CsrGraphBuilder::new]\r\n");
                return None;
            }
        };
        
        // Build ID -> index map
        let mut id_to_idx: [(usize, usize); 128] = [(0, 0); 128];
        let node_count = nodes.len().min(128);
        
        for (i, (id, label)) in nodes.iter().enumerate() {
            if builder.add_node(*id, label).is_none() {
                write_serial(serial, usb_dev, "  [Arena FAIL: add_node]\r\n");
                return None;
            }
            if i < 128 {
                id_to_idx[i] = (*id, i);
            }
        }
        
        let find_idx = |id: usize| -> Option<usize> {
            for i in 0..node_count {
                if id_to_idx[i].0 == id {
                    return Some(id_to_idx[i].1);
                }
            }
            None
        };
        
        for (u, v) in edges { 
            let from_idx = find_idx(*u)?;
            let to_idx = find_idx(*v)?;
            if builder.add_edge(from_idx, to_idx).is_none() {
                write_serial(serial, usb_dev, "  [Arena FAIL: add_edge]\r\n");
                return None;
            }
        }
        
        let graph = builder.build()?;
        let build_time = timer.get_counter() - build_start;
        
        // Phase 2: Compute layout
        let compute_start = timer.get_counter();
        let mut layout_temp_arena = Arena::new(layout_temp_mem);
        let mut final_output_arena = Arena::new(output_mem);
        
        let layout = match graph.compute_layout_arena(&ascii_dag::algorithms::sugiyama::config::LayoutConfig::standard(), &mut layout_temp_arena, &mut final_output_arena) {
            Some(l) => l,
            None => {
                write_serial(serial, usb_dev, "  [Arena FAIL: compute_layout]\r\n");
                return None;
            }
        };
        let compute_time = timer.get_counter() - compute_start;
        
        // Measure arena usage
        let temp_used = layout_temp_arena.used();
        
        // Phase 3: Render
        let render_start = timer.get_counter();
        let mut render_buf = [0u8; 8192]; // Larger buffer for 100 nodes
        let mut line_buf = [' '; 512];
        let _rendered_len = layout.render_to_buffer(&mut render_buf, &mut line_buf).unwrap_or(0);
        let render_time = timer.get_counter() - render_start;
        
        Some(BenchResult {
            build_us: build_time.ticks(),
            compute_us: compute_time.ticks(),
            render_us: render_time.ticks(),
            memory_bytes: temp_used,
        })
    }
}
