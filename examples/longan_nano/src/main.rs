#![no_std]
#![no_main]

use panic_halt as _;

use embedded_graphics::mono_font::{ascii::FONT_4X6, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use longan_nano::hal::{pac, prelude::*};
use longan_nano::{lcd, lcd_pins};
use riscv_rt::entry;

use ascii_dag::algorithms::sugiyama::config::LayoutConfig;
use ascii_dag::graph::Direction;
use ascii_dag::graph::arena::Arena;
use ascii_dag::graph::csr::CsrGraphBuilder;
use ascii_dag::render::engine::RenderOptions;

/// Render a DAG on the Longan Nano's 160×80 LCD.
///
/// Pure arena mode — no heap allocator, every buffer on the stack.
/// LCD: 160×80 px, FONT_4X6 → 40 chars × 13 lines.
///
/// Two choices matter on this display:
///
/// * **ASCII charset.** `FONT_4X6` has no box-drawing glyphs, so the
///   render uses `RenderOptions::ascii()` — the same canvas decoded
///   through the ASCII table (`+ - | >` instead of `┌ ─ │ →`).
/// * **LeftRight direction.** 40 columns × 13 lines is a wide, short
///   window, and a chain laid out top-down runs out of lines long
///   before it runs out of columns. LR turns levels into columns and
///   fits the same graph in a few rows.
#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();

    // Configure clocks
    let mut rcu = dp
        .RCU
        .configure()
        .ext_hf_clock(8.mhz())
        .sysclk(108.mhz())
        .freeze();
    let mut afio = dp.AFIO.constrain(&mut rcu);

    // Setup LCD
    let gpioa = dp.GPIOA.split(&mut rcu);
    let gpiob = dp.GPIOB.split(&mut rcu);
    let lcd_pins = lcd_pins!(gpioa, gpiob);
    let mut lcd = lcd::configure(dp.SPI0, lcd_pins, &mut afio, &mut rcu);
    let (width, height) = (lcd.size().width as u32, lcd.size().height as u32);

    // Clear screen to black
    Rectangle::new(Point::new(0, 0), Size::new(width, height))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(&mut lcd)
        .unwrap();

    // Text styles
    let title_style = MonoTextStyleBuilder::new()
        .font(&FONT_4X6)
        .text_color(Rgb565::new(0, 50, 31)) // cyan-ish
        .background_color(Rgb565::BLACK)
        .build();
    let dag_style = MonoTextStyleBuilder::new()
        .font(&FONT_4X6)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::BLACK)
        .build();

    // Title
    Text::new("ascii-dag LR/ascii (no_alloc)", Point::new(2, 6), title_style)
        .draw(&mut lcd)
        .unwrap();

    // ── Arena memory on stack ──────────────────────────────
    let mut graph_buf = [0u8; 1024];
    let mut output_buf = [0u8; 2048];
    let mut temp_buf = [0u8; 2048];
    // The render engine carves its plan + band canvas from an arena of
    // its own, and writes text into a plain byte buffer.
    let mut render_buf = [0u8; 4096];
    let mut text_buf = [0u8; 2048];

    // ── Build graph ────────────────────────────────────────
    let mut graph_arena = Arena::new(&mut graph_buf);
    let mut builder = CsrGraphBuilder::new(&mut graph_arena, 5, 5, 64, 0).unwrap();

    let n0 = builder.add_node(0, "Init").unwrap();
    let n1 = builder.add_node(1, "Build").unwrap();
    let n2 = builder.add_node(2, "Test").unwrap();
    let n3 = builder.add_node(3, "Deploy").unwrap();

    builder.add_edge(n0, n1).unwrap();
    builder.add_edge(n1, n2).unwrap();
    builder.add_edge(n2, n3).unwrap();
    builder.add_edge(n0, n2).unwrap(); // skip-level edgeascii-da

    let graph = builder.build().unwrap();

    // ── Compute layout ─────────────────────────────────────
    // Levels become columns: the chain runs across the 40-column
    // display instead of down its 13 lines.
    let mut config = LayoutConfig::standard();
    config.direction = Direction::LeftRight;

    let mut output_arena = Arena::new(&mut output_buf);
    let layout = {
        let mut temp_arena = Arena::new(&mut temp_buf);
        graph.compute_layout_arena(&config, &mut temp_arena, &mut output_arena)
    };

    match layout {
        Ok(ir) => {
            // ASCII charset: FONT_4X6 has no box-drawing glyphs.
            let options = RenderOptions::ascii();
            let render_arena = Arena::new(&mut render_buf);

            if let Ok(bytes) = ir.render_to_bytes(&options, &render_arena, &mut text_buf) {
                if let Ok(text) = core::str::from_utf8(&text_buf[..bytes]) {
                    // Draw DAG output line by line
                    // Title at y=6, leave gap, start DAG at y=14
                    let mut y = 14;
                    for line in text.lines().take(11) {
                        Text::new(line, Point::new(2, y), dag_style)
                            .draw(&mut lcd)
                            .unwrap();
                        y += 6;
                    }
                }
            } else {
                Text::new("Render OOM", Point::new(2, 20), dag_style)
                    .draw(&mut lcd)
                    .unwrap();
            }
        }
        Err(_) => {
            Text::new("Layout OOM", Point::new(2, 20), dag_style)
                .draw(&mut lcd)
                .unwrap();
        }
    }

    loop {
        unsafe { riscv::asm::wfi() };
    }
}
