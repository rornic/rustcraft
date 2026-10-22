use std::{
    f32::consts::TAU,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use bevy::{
    asset::{Asset, Assets},
    audio::{AudioPlayer, ChannelCount, Decodable, PlaybackSettings, SampleRate, Source},
    ecs::{
        resource::Resource,
        system::{Commands, Res, ResMut},
    },
    reflect::TypePath,
    time::Time,
};

use super::{smoothstep, Surroundings};

/// What the wind's sound follows, set as the camera moves and read on the audio thread.
#[derive(Resource, Clone, Default)]
pub struct WindDials {
    // how hard the wind blows where the camera is, 0..1
    strength: Dial,
    // how much of a breeze the sound is blowing right now, 0..1, whatever its strength: set
    // on the audio thread, for the leaves to rustle in, see `Loop::Leaves`
    breeze: Dial,
}

impl WindDials {
    pub(super) fn breeze(&self) -> f32 {
        self.breeze.get()
    }
}

/// An f32 shared between threads through its bits, so the sounds made as they play can read
/// what they follow, see `weather::RainSound` too.
#[derive(Clone, Default)]
pub(crate) struct Dial(Arc<AtomicU32>);

impl Dial {
    pub(crate) fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub(crate) fn set(&self, strength: f32) {
        self.0.store(strength.to_bits(), Ordering::Relaxed);
    }
}

// How strong a breeze on the ground is, when one blows, see `SPELL_SECONDS`.
const GROUND_BREEZE: f32 = 0.1;
// How far above the ground or sea the wind reaches full strength, in blocks.
const FULL_WIND_ALTITUDE: f32 = 250.0;
// How fast the air rushes past a camera flying flat out, in blocks a second, adding to the wind.
const RUSH_SPEED: f32 = 80.0;
const RUSH_GAIN: f32 = 0.15;
// How strong the wind blows on the ground in the heaviest rain: strong enough to keep blowing
// between spells, see `STEADY`.
const STORM_WIND: f32 = 0.6;
// Underwater the wind is only a murmur through the surface.
const UNDERWATER_WIND: f32 = 0.02;
// How long the wind takes to follow a change in where the camera is, in seconds.
const WIND_FOLLOW_SECONDS: f32 = 0.6;

pub fn play_wind(mut commands: Commands, mut winds: ResMut<Assets<Wind>>, dials: Res<WindDials>) {
    let wind = winds.add(Wind {
        dials: dials.clone(),
    });
    commands.spawn((AudioPlayer(wind), PlaybackSettings::ONCE));
}

/// The wind's strength from how high the camera is over the ground or sea, and how fast it's
/// flying.
pub fn wind_strength(altitude: f32, speed: f32) -> f32 {
    // slow to pick up off the ground, so even treetop height is calm
    let height = (altitude / FULL_WIND_ALTITUDE).clamp(0.0, 1.0).powf(1.5);
    let rush = (speed / RUSH_SPEED).clamp(0.0, 1.0) * RUSH_GAIN;
    (GROUND_BREEZE + (1.0 - GROUND_BREEZE) * height + rush).min(1.0)
}

pub fn update_wind(
    time: Res<Time>,
    around: Res<Surroundings>,
    dials: Res<WindDials>,
    weather: Option<Res<crate::weather::Weather>>,
) {
    let storm = weather.map_or(0.0, |weather| weather.rain) * STORM_WIND;
    let wanted = if around.underwater {
        UNDERWATER_WIND
    } else {
        wind_strength(around.altitude, around.speed).max(storm)
    };
    let follow = 1.0 - (-time.delta_secs() / WIND_FOLLOW_SECONDS).exp();
    let current = dials.strength.get();
    dials.strength.set(current + (wanted - current) * follow);
}

/// Wind made from noise as it plays, so it never loops: a low roar that swells with the
/// gusts, and a whistle that rises in pitch as they blow harder.
#[derive(Asset, TypePath)]
pub struct Wind {
    dials: WindDials,
}

impl Decodable for Wind {
    type Decoder = WindDecoder;

    fn decoder(&self) -> WindDecoder {
        WindDecoder::new(self.dials.clone(), rand::random())
    }
}

pub(crate) const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
// How often the decoder rereads the strength and moves its gusts on, in frames.
const CONTROL_FRAMES: u32 = 64;
// How long the decoder takes to follow the strength, in seconds, so steps in it don't click.
const STRENGTH_GLIDE_SECONDS: f32 = 0.3;
// Wind comes in spells: how long one takes to pick up or die away, in seconds, at the least
// and most. Down low it drops to nothing between them, see `CALM`.
const SPELL_SECONDS: (f32, f32) = (12.0, 40.0);
// Where a spell's wandering value, 0..1, goes from still air (under the first) to a breeze
// that's fully up (over the second): about a third of the time it's still.
const CALM: (f32, f32) = (0.35, 0.8);
// Wind this strong, from high up or flying fast, keeps blowing between spells: from none of
// it at the first to all of it at the second.
const STEADY: (f32, f32) = (0.35, 0.8);
// How long one gust takes to rise or die away, in seconds, at the least and most.
const GUST_SECONDS: (f32, f32) = (2.5, 7.0);
// The roar's low-pass cutoff in calm air and in the strongest gusts, in Hz. Wind is mostly
// rumble: anything much higher hisses like a detuned radio.
const ROAR_CUTOFF: (f32, f32) = (200.0, 600.0);
// One-pole stages in the roar's low-pass, each cutting another 6 dB an octave past the cutoff.
const ROAR_POLES: usize = 3;
// The whistle's pitch in calm air and in the strongest gusts, in Hz.
const WHISTLE_PITCH: (f32, f32) = (260.0, 820.0);
// Narrow, so the whistle is a note rather than a band of hiss.
const WHISTLE_Q: f32 = 14.0;
// Cuts the whistle's input, so it only rings with the rumble's low end too.
const WHISTLE_INPUT_CUTOFF: f32 = 1200.0;
// Quieter than the roar, and only once the wind picks up, see `WindDecoder::control`.
const WHISTLE_GAIN: f32 = 0.2;
const OUTPUT_GAIN: f32 = 0.5;
// Puts a breeze on the ground at about -37 LUFS, the quietest thing there, and a gale high up
// at about -22, see `Loop::loudness`.
const ROAR_MAKEUP: f32 = 0.34;

pub struct WindDecoder {
    dials: WindDials,
    rng: u64,
    frame: u32,
    channel: u16,
    // the strength as heard, gliding after `strength`
    level: f32,
    spell: Gust,
    gust: Gust,
    flutter: Gust,
    // per channel, so the two ears hear different air
    roar: [[OnePole; ROAR_POLES]; 2],
    whistle_input: [OnePole; 2],
    whistle: [Bandpass; 2],
    // how much of each the control step last asked for; the roar's also makes up for what
    // its low-pass takes, so moving the cutoff changes how bright it is, not how loud
    roar_gain: f32,
    whistle_gain: f32,
}

impl WindDecoder {
    fn new(dials: WindDials, seed: u64) -> Self {
        let level = dials.strength.get();
        let mut decoder = Self {
            dials,
            rng: seed | 1,
            frame: 0,
            channel: 0,
            level,
            spell: Gust::default(),
            gust: Gust::default(),
            flutter: Gust::default(),
            roar: [[OnePole::default(); ROAR_POLES]; 2],
            whistle_input: [OnePole::default(); 2],
            whistle: [Bandpass::default(); 2],
            roar_gain: 0.0,
            whistle_gain: 0.0,
        };
        for filter in &mut decoder.whistle_input {
            filter.tune(WHISTLE_INPUT_CUTOFF);
        }
        decoder.control();
        decoder
    }

    // xorshift: plenty for noise, and cheap enough to run every sample
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn noise(&mut self) -> f32 {
        self.random() * 2.0 - 1.0
    }

    // Moves the gusts on and retunes the filters, every `CONTROL_FRAMES`.
    fn control(&mut self) {
        let step = CONTROL_FRAMES as f32 / SAMPLE_RATE as f32;
        let glide = 1.0 - (-step / STRENGTH_GLIDE_SECONDS).exp();
        self.level += (self.dials.strength.get() - self.level) * glide;
        let (a, b) = (self.random(), self.random());
        let spell = self.spell.advance(step, SPELL_SECONDS, a, b);
        let (a, b) = (self.random(), self.random());
        let gust = self.gust.advance(step, GUST_SECONDS, a, b);
        let (a, b) = (self.random(), self.random());
        let flutter = self.flutter.advance(step, (0.2, 0.8), a, b);

        let up = smoothstep(CALM.0, CALM.1, spell);
        let steady = smoothstep(STEADY.0, STEADY.1, self.level);
        let blowing = up + (1.0 - up) * steady;
        let gusting = 0.65 + 0.35 * gust;
        self.dials.breeze.set(blowing * gusting);
        // gusts swing a calm day's wind far less than a gale's
        let blow = (self.level * blowing * gusting * (0.9 + 0.1 * flutter)).clamp(0.0, 1.0);
        let cutoff = lerp(ROAR_CUTOFF, blow);
        let pitch = lerp(WHISTLE_PITCH, blow * (0.85 + 0.15 * flutter));
        for channel in 0..2 {
            for pole in &mut self.roar[channel] {
                pole.tune(cutoff);
            }
            // the ears' whistles a little apart, so it doesn't sit in the middle of the head
            self.whistle[channel].tune(pitch * (1.0 + 0.04 * channel as f32), WHISTLE_Q);
        }
        // noise through a low-pass keeps about the square root of its cutoff's share of power
        let makeup = (SAMPLE_RATE as f32 / 2.0 / cutoff).sqrt();
        self.roar_gain = blow.powf(0.95) * makeup * ROAR_MAKEUP;
        self.whistle_gain = WHISTLE_GAIN * blow * blow;
    }
}

impl Iterator for WindDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.channel == 0 {
            if self.frame.is_multiple_of(CONTROL_FRAMES) {
                self.control();
            }
            self.frame = self.frame.wrapping_add(1);
        }
        let channel = self.channel as usize;
        self.channel = (self.channel + 1) % CHANNELS;

        let white = self.noise();
        let roar = self.roar[channel]
            .iter_mut()
            .fold(white, |signal, pole| pole.process(signal));
        let whistle_input = self.whistle_input[channel].process(white);
        let whistle = self.whistle[channel].process(whistle_input);
        let sample = roar * self.roar_gain + whistle * self.whistle_gain;
        Some((sample * OUTPUT_GAIN).tanh())
    }
}

impl Source for WindDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(CHANNELS).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn lerp((from, to): (f32, f32), t: f32) -> f32 {
    from + (to - from) * t
}

/// A smoothly wandering value, 0..1: it eases from one random target to the next, each
/// taking a random time within the given range.
#[derive(Default)]
pub(super) struct Gust {
    from: f32,
    to: f32,
    progress: f32,
    seconds: f32,
}

impl Gust {
    // `a` and `b` are fresh random numbers, 0..1, used only when a new target is picked.
    pub(super) fn advance(
        &mut self,
        step: f32,
        (shortest, longest): (f32, f32),
        a: f32,
        b: f32,
    ) -> f32 {
        if self.progress >= 1.0 || self.seconds == 0.0 {
            self.from = self.to;
            self.to = a;
            self.seconds = shortest + (longest - shortest) * b;
            self.progress = 0.0;
        }
        self.progress = (self.progress + step / self.seconds).min(1.0);
        let eased = ochre_base::math::ease(self.progress);
        self.from + (self.to - self.from) * eased
    }
}

#[derive(Default, Clone, Copy)]
struct OnePole {
    coefficient: f32,
    state: f32,
}

impl OnePole {
    fn tune(&mut self, cutoff: f32) {
        self.coefficient = 1.0 - (-TAU * cutoff / SAMPLE_RATE as f32).exp();
    }

    fn process(&mut self, input: f32) -> f32 {
        self.state += (input - self.state) * self.coefficient;
        self.state
    }
}

/// A state variable filter's band-pass output.
#[derive(Default, Clone, Copy)]
struct Bandpass {
    f: f32,
    damping: f32,
    low: f32,
    band: f32,
}

impl Bandpass {
    fn tune(&mut self, centre: f32, q: f32) {
        self.f = 2.0 * (std::f32::consts::PI * centre / SAMPLE_RATE as f32).sin();
        self.damping = 1.0 / q;
    }

    fn process(&mut self, input: f32) -> f32 {
        self.low += self.f * self.band;
        let high = input - self.low - self.damping * self.band;
        self.band += self.f * high;
        self.band
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wind_grows_with_altitude_and_speed_and_never_goes_silent() {
        assert_eq!(GROUND_BREEZE, wind_strength(0.0, 0.0));
        assert_eq!(GROUND_BREEZE, wind_strength(-20.0, 0.0));
        assert!(wind_strength(50.0, 0.0) > wind_strength(10.0, 0.0));
        assert_eq!(1.0, wind_strength(FULL_WIND_ALTITUDE * 2.0, 0.0));
        assert!(wind_strength(0.0, RUSH_SPEED) > wind_strength(0.0, 0.0));
        assert_eq!(1.0, wind_strength(FULL_WIND_ALTITUDE, RUSH_SPEED));
    }

    // The loudness of each second of two minutes of wind this strong.
    fn loudness(strength: f32) -> Vec<f32> {
        let shared = WindDials::default();
        shared.strength.set(strength);
        let second = SAMPLE_RATE as usize * CHANNELS as usize;
        let samples: Vec<f32> = WindDecoder::new(shared, 7).take(second * 120).collect();
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        samples
            .chunks(second)
            .map(|s| (s.iter().map(|s| s * s).sum::<f32>() / s.len() as f32).sqrt())
            .collect()
    }

    #[test]
    fn test_a_breeze_comes_and_goes_but_a_gale_keeps_blowing() {
        let breeze = loudness(GROUND_BREEZE);
        let loudest = breeze.iter().cloned().fold(0.0, f32::max);
        assert!(loudest > 0.005, "loudest {loudest}");
        assert!(breeze.iter().any(|&l| l < loudest * 0.05), "{breeze:?}");
        let gale = loudness(1.0);
        let quietest = gale.iter().cloned().fold(1.0, f32::min);
        assert!(quietest > loudest, "quietest {quietest}, breeze {loudest}");
    }
}
