use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::fs::File;
use std::num::{NonZeroU16, NonZeroUsize};
use std::ops::Range;
use std::thread;
use std::thread::available_parallelism;
use image::codecs::gif::{GifDecoder, GifEncoder, Repeat};
use image::{AnimationDecoder, Frame, RgbaImage};
use imageproc::drawing::draw_filled_circle_mut;
use rand::{Rng, SeedableRng};
use log::{Level, LevelFilter};
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Logger, Root};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::Config;
use rand_pcg::Pcg64Mcg;
use rayon::ThreadPoolBuilder;
use world::cpu::CPUWorld;
use world::gpu::GPUWorld;
use crate::periodic_logger::PeriodicLogger;
use crate::vector::Vector;
use crate::world::{MassPoint, Particle};
use crate::world::par::ParWorld;

mod vector;
mod periodic_logger;
mod world;

const SEED: u64 = 23;
const PARTICLE_COUNT: usize = 100;
const FRAME_COUNT: usize = 240;
const SCALE: f32 = 500.0;
const TIME_PER_FRAME: f32 = 20.0;
const TIME_STEPS: NonZeroU16 = match NonZeroU16::new(20) {
    None => panic!("TIME_STEPS may not be 0"),
    Some(steps) => steps
};
const SIZE: Option<(f32, f32)> = Some((1000.0, 1000.0));
const PARTICLE_GENERATOR: fn() -> Vec<Particle> = generate_particles;

fn main() {
    initialize_logging();

    compare_outputs();
}

#[allow(dead_code)]
fn output_gpu() {
    let world = GPUWorld::new(PARTICLE_GENERATOR());
    tick_and_output_gif(world, GPUWorld::tick, GPUWorld::get_mass_points, "gpu");
}

#[allow(dead_code)]
fn compare_outputs() {
    let particles = PARTICLE_GENERATOR();
    let particles_a = particles.clone();
    let particles_b = particles.clone();
    let particles_c = particles;

    ThreadPoolBuilder::new()
    .num_threads(
    usize::max(
    available_parallelism()
    .unwrap_or(NonZeroUsize::new(1).unwrap())
    .get() - 1,
    1)
    )
    .build_global()
    .unwrap();
    let handles = [
    thread::spawn(|| {
        let world = CPUWorld { particles: particles_a };
        tick_and_output_gif(world, CPUWorld::tick, CPUWorld::get_mass_points, "cpu");
    }),
    thread::spawn(|| {
        let world = ParWorld::new(particles_b);
        tick_and_output_gif(world, ParWorld::tick, ParWorld::get_mass_points, "par");
    }),
    thread::spawn(|| {
        let world = GPUWorld::new(particles_c);
        tick_and_output_gif(world, GPUWorld::tick, GPUWorld::get_mass_points, "gpu");
    })
    ];
    for handle in handles {
        handle.join().unwrap();
    }

    {
        let single = GifDecoder::new(File::open("output/cpu.gif").unwrap()).unwrap();
        let multi = GifDecoder::new(File::open("output/par.gif").unwrap()).unwrap();
        let gpu = GifDecoder::new(File::open("output/gpu.gif").unwrap()).unwrap();
        let mut merged = GifEncoder::new(File::create("output/merged.gif").unwrap());
        merged.set_repeat(Repeat::Infinite).unwrap();
        let mut periodic_logger = PeriodicLogger::new("exporting merged", Level::Info);
        single.into_frames()
        .zip(multi.into_frames())
        .zip(gpu.into_frames())
        .map(|((single_frame_result, multi_frame_result), gpu_frame_result)| {
            (single_frame_result.unwrap().into_buffer(), multi_frame_result.unwrap().into_buffer(), gpu_frame_result.unwrap().into_buffer())
        })
        .enumerate()
        .for_each(|(frame, (single_frame, multi_frame, gpu_frame))| {
            periodic_logger.log(format!("{} / {}", frame, FRAME_COUNT));
            let (width, height) = (single_frame.width(), single_frame.height());
            let mut image = RgbaImage::new(width, height);
            for y in 0..height {
                for x in 0..width {
                    let r = single_frame[(x, y)].0[0];
                    let g = multi_frame[(x, y)].0[1];
                    let b = gpu_frame[(x, y)].0[2];
                    image[(x, y)].0 = [r, g, b, 255];
                }
            }
            merged.encode_frame(Frame::new(image)).unwrap();
        });
    }
}

#[allow(dead_code)]
fn generate_particles() -> Vec<Particle> {
    let mut rng = Pcg64Mcg::seed_from_u64(SEED);
    let mut particles = Vec::with_capacity(PARTICLE_COUNT);
    for _ in 0..PARTICLE_COUNT {
        particles.push(Particle {
            mass: rng.gen_range(0.0..1.0),
            position: Vector::new(rng.gen_range(0.0..TAU), rng.gen_range(0.5..1.0)),
            velocity: Vector::new(0.0, 0.0)
        });
    }
    particles
}

#[allow(dead_code)]
fn generate_3_body() -> Vec<Particle> {
    let mut particles = Vec::with_capacity(PARTICLE_COUNT);
    particles.push(Particle {
        mass: 10000.0,
        position: Vector::new(0.0, 0.0),
        velocity: Vector::new(0.0, 0.0)
    });
    particles.push(Particle {
        mass: 100.0,
        position: Vector::new(0.0, 0.50),
        velocity: Vector::new(FRAC_PI_2, 0.001)
    });
    particles.push(Particle {
        mass: 10.0,
        position: Vector::new(0.0, 0.55),
        velocity: Vector::new(FRAC_PI_2, 0.0013)
    });
    particles
}

fn tick_and_output_gif<W, TF: FnMut(&mut W, f32, NonZeroU16), MPG: FnMut(&W) -> Vec<MassPoint>>(mut world: W, mut tick_function: TF, mut mass_point_getter: MPG, name: &str) {
    let mut periodic_logger = PeriodicLogger::new(&format!("simulating {}", name), Level::Info);
    let mut mass_position_frames = Vec::with_capacity(FRAME_COUNT);
    for frame in 0..FRAME_COUNT {
        tick_function(&mut world, TIME_PER_FRAME, TIME_STEPS);
        mass_position_frames.push(mass_point_getter(&world));
        periodic_logger.log(format!("{} / {}", frame, FRAME_COUNT));
    }
    output_gif(mass_position_frames, name);
}

fn output_gif(mass_position_frames: Vec<Vec<MassPoint>>, name: &str) {
    let mut bounds_x;
    let mut bounds_y;
    let mut bounds_mass;
    if let Some((width, height)) = SIZE {
        let w = (width - 1.0) / 2.0 / SCALE;
        let h = (height - 1.0) / 2.0 / SCALE;
        bounds_x = -w..w;
        bounds_y = -h..h;
        bounds_mass = 0.0..1000.0;
    } else {
        {
            let MassPoint { mass, position: (x, y) } = mass_position_frames[0][0];
            bounds_x = x..x;
            bounds_y = y..y;
            bounds_mass = mass..mass;
        }
        for mass_positions in &mass_position_frames {
            for mass_position in mass_positions {
                let MassPoint { mass, position: (x, y) } = *mass_position;
                adjust_bounds(&mut bounds_mass, mass);
                adjust_bounds(&mut bounds_x, x);
                adjust_bounds(&mut bounds_y, y);
            }
        }
    }


    let width = ((bounds_x.end - bounds_x.start) * SCALE) as u32 + 1;
    let height = ((bounds_y.end - bounds_y.start) * SCALE) as u32 + 1;
    let mut gif = GifEncoder::new(
        File::create(format!("output/{}.gif", name))
            .expect("unable to create file")
    );
    gif.set_repeat(Repeat::Infinite)
        .expect("unable to make gif infinitely repeatable");
    let mut periodic_logger = PeriodicLogger::new(&format!("exporting {}", name), Level::Info);
    for (frame, mass_positions) in mass_position_frames.iter().enumerate() {
        let mut image = RgbaImage::new(width, height);
        for pixel in image.pixels_mut() {
            pixel.0 = [0, 0, 0, 255];
        }
        for mass_position in mass_positions {
            let MassPoint { mass, position: (x, y) } = mass_position;
            let px = ((x - bounds_x.start) * SCALE) as i32;
            let py = ((y - bounds_y.start) * SCALE) as i32;
            //let m = ((1.0 - mass / bounds_mass.end) * 255.0) as u8;
            draw_filled_circle_mut(
                &mut image,
                (px, py),
                f32::cbrt(3.0 * mass / 4.0 * PI) as i32,
                [255, 255, 255, 255].into()
            );
        }
        gif.encode_frame(Frame::new(image))
            .expect("error occurred while encoding frame");
        periodic_logger.log(format!("{} / {}", frame, FRAME_COUNT));
    }
}

fn adjust_bounds(bounds: &mut Range<f32>, v: f32) {
    if v < bounds.start {
        bounds.start = v;
    } else if v > bounds.end {
        bounds.end = v;
    }
}

#[deny(dead_code)]
fn initialize_logging() {
    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{m}{n}")))
        .build();

    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .logger(Logger::builder().build("app::backend::db", LevelFilter::Info))
        .build(Root::builder().appender("stdout").build(LevelFilter::Info))
        .unwrap();

    log4rs::init_config(config).unwrap();
}
