//! The pad's voice — the eight notes a code is made of, synthesised rather than shipped.
//!
//! Eight short plucks: four directions, each in a low octave for the arm press and a high
//! one for the key press. They are built out of arithmetic at startup because that is the
//! smaller thing to keep true — a folder of eight sound files is eight files that can go
//! missing, be recorded at different volumes, or drift out of tune with
//! [`Dir::hz`](crate::code::Dir::hz), and none of that is possible when the pitch the pad
//! is drawn from is the pitch that gets played.
//!
//! The tone is a pluck: a fast attack, an exponential decay, and one octave of overtone on
//! top so it reads as an instrument rather than a beep. Short on purpose — a code is two
//! presses in a fifth of a second when a practised thumb types it, and the notes have to
//! stay out of each other's way at that speed.

use std::sync::Arc;

use bevy::audio::AudioSource;
use bevy::prelude::*;

use crate::code::{Dir, Pad};

const RATE: u32 = 44_100;
/// How long one note rings. Long enough to have a pitch, short enough that a fast code is
/// two notes and not a chord.
const SECONDS: f32 = 0.24;
/// Peak of the envelope, well under full scale: this plays over a game, sixty times a
/// minute, next to a child.
const LEVEL: f32 = 0.28;
/// Seconds of the attack. Instant is a click; this is a pluck.
const ATTACK: f32 = 0.006;
/// How much of the note has decayed away by the end — the shape that makes it a struck
/// string rather than a held organ pipe.
const DECAY: f32 = 6.5;

/// The eight notes, by direction and octave. Built once, played by handle.
#[derive(Resource)]
pub struct Voice([[Handle<AudioSource>; 2]; 4]);

impl Voice {
    fn note(&self, dir: Dir, high: bool) -> Handle<AudioSource> {
        self.0[dir.index()][high as usize].clone()
    }
}

/// Builds the eight notes. Runs once, before anything can press anything.
pub fn tune(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    let mut note = |dir: Dir, high: bool| {
        sources.add(AudioSource {
            bytes: Arc::from(wav(dir.hz(high)).into_boxed_slice()),
        })
    };
    commands.insert_resource(Voice(
        Dir::ALL.map(|dir| [note(dir, false), note(dir, true)]),
    ));
}

/// Sounds whatever the pad struck this frame.
///
/// The pad pushes notes and this drains them, exactly as the rig pushes crafts and the
/// host drains those: a drawing that owned a speaker would be a drawing that could not be
/// filmed on a machine without one.
pub fn play(mut commands: Commands, voice: Res<Voice>, mut pad: ResMut<Pad>) {
    for note in pad.sounded.drain(..).collect::<Vec<_>>() {
        commands.spawn((
            AudioPlayer(voice.note(note.dir, note.high)),
            PlaybackSettings::DESPAWN,
        ));
    }
}

/// One plucked note, as a mono 16-bit WAV.
///
/// A whole file rather than raw samples because that is the format bevy's audio already
/// knows how to decode; the alternative is a custom decoder for eight fifths of a second
/// of sound.
fn wav(hz: f32) -> Vec<u8> {
    let samples = (RATE as f32 * SECONDS) as u32;
    let mut pcm = Vec::with_capacity(samples as usize * 2);
    for i in 0..samples {
        let t = i as f32 / RATE as f32;
        let attack = (t / ATTACK).min(1.0);
        let body = (-DECAY * t / SECONDS).exp();
        let tone = (std::f32::consts::TAU * hz * t).sin()
            + 0.35 * (std::f32::consts::TAU * 2.0 * hz * t).sin();
        let v = LEVEL * attack * body * tone;
        pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }

    let mut out = Vec::with_capacity(44 + pcm.len());
    let bytes_per_second = RATE * 2;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // the size of this header
    out.extend_from_slice(&1u16.to_le_bytes()); // uncompressed
    out.extend_from_slice(&1u16.to_le_bytes()); // one channel
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&bytes_per_second.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

#[cfg(test)]
mod tests {
    use bevy::audio::Decodable;

    use super::*;

    /// The bytes really are a sound file, decoded by the same decoder that will be asked
    /// to play them. A hand-written header is exactly the kind of thing that is wrong by
    /// four bytes and only says so the first time a child presses a button.
    #[test]
    fn every_note_decodes() {
        for dir in Dir::ALL {
            for high in [false, true] {
                let source = AudioSource {
                    bytes: Arc::from(wav(dir.hz(high)).into_boxed_slice()),
                };
                let samples = source.decoder().count();
                assert!(
                    samples > (RATE as f32 * SECONDS * 0.9) as usize,
                    "{dir:?} high={high} decoded to {samples} samples"
                );
            }
        }
    }

    /// A note starts and ends at silence. One that starts at full deflection clicks, and
    /// one that is cut off mid-swing clicks on the way out — at two notes a code and a
    /// code every few seconds, both are audible all afternoon.
    #[test]
    fn a_note_fades_in_and_out() {
        let bytes = wav(440.0);
        let pcm = &bytes[44..];
        let at = |i: usize| i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]).abs() as f32;
        let peak = (0..pcm.len() / 2).map(at).fold(0.0, f32::max);
        assert!(peak > 3000.0, "the note is inaudible: peak {peak}");
        assert!(at(0) < peak * 0.05, "it starts with a click");
        assert!(at(pcm.len() / 2 - 1) < peak * 0.05, "it ends with a click");
    }
}
