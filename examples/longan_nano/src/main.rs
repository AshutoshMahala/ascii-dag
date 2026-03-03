#![no_std]
#![no_main]

use panic_halt as _; 
extern crate alloc;
use embedded_alloc::Heap;
use longan_nano::hal::{prelude::*, pac};
use longan_nano::{lcd, lcd_pins};
use riscv_rt::entry;
use ascii_dag::Graph;

use embedded_graphics::mono_font::{ascii::FONT_4X6, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Rectangle, PrimitiveStyle};
use embedded_graphics::text::Text;

#[global_allocator]
static HEAP: Heap = Heap::empty();

#[entry]
fn main() -> ! {
    // Init Heap
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 8192; // 8KB
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }

    let dp = pac::Peripherals::take().unwrap();
    
    // Configure clocks
    let mut rcu = dp.RCU.configure().ext_hf_clock(8.mhz()).sysclk(108.mhz()).freeze();
    
    // Setup AFIO
    let mut afio = dp.AFIO.constrain(&mut rcu);
    
    // Setup GPIO for LCD
    let gpioa = dp.GPIOA.split(&mut rcu);
    let gpiob = dp.GPIOB.split(&mut rcu);
    
    // Initialize LCD (160x80 pixels)
    let lcd_pins = lcd_pins!(gpioa, gpiob);
    let mut lcd = lcd::configure(dp.SPI0, lcd_pins, &mut afio, &mut rcu);
    let (width, height) = (lcd.size().width as i32, lcd.size().height as i32);
    
    // Clear screen to black
    Rectangle::new(Point::new(0, 0), Size::new(width as u32, height as u32))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(&mut lcd)
        .unwrap();

    // Build and render DAG
    let mut dag = Graph::new();
    dag.add_node(1, "Init");
    dag.add_node(2, "Build");
    dag.add_node(3, "Test");
    dag.add_node(4, "Deploy");
    dag.add_edge(1, 2, None);
    dag.add_edge(2, 3, None);
    dag.add_edge(3, 4, None);

    let output = dag.render();
    
    // Text style - tiny font for 160x80 LCD
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_4X6)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::BLACK)
        .build();
    
    // Draw title
    Text::new("ascii-dag on Longan!", Point::new(2, 8), style)
        .draw(&mut lcd)
        .unwrap();
    
    // Draw DAG output line by line
    let mut y = 18;
    for line in output.lines().take(10) { // LCD is small, limit lines
        Text::new(line, Point::new(2, y), style)
            .draw(&mut lcd)
            .unwrap();
        y += 7;
    }

    loop {}
}
