#!/usr/bin/env python3
"""Writes score.wav beside this file: the showcase's music, synthesized, so no
sample or track needs a licence. 120 BPM: a beat is 30 frames of the film.
Run it once before rendering; the film plays silent without it."""
import array
import math
import pathlib
import random
import wave

RATE = 48000
FPS = 60
SECONDS = 33
FRAMES = RATE * SECONDS
BEAT = RATE // 2
left = array.array("f", [0.0]) * FRAMES
right = array.array("f", [0.0]) * FRAMES
rng = random.Random(7)


def add(start, samples, pan=0.5, gain=1.0):
    gl, gr = gain * math.cos(pan * math.pi / 2), gain * math.sin(pan * math.pi / 2)
    for i, s in enumerate(samples):
        at = start + i
        if at >= FRAMES:
            break
        left[at] += s * gl
        right[at] += s * gr


def midi(n):
    return 440.0 * 2 ** ((n - 69) / 12)


def kick(gain=1.0):
    out, phase = [], 0.0
    for i in range(int(RATE * 0.32)):
        t = i / RATE
        phase += 2 * math.pi * (46 + 130 * math.exp(-t * 34)) / RATE
        out.append(math.sin(phase) * math.exp(-t * 11) * gain)
    return out


def hat(gain=1.0, length=0.05):
    out, last = [], 0.0
    for i in range(int(RATE * length)):
        n = rng.uniform(-1, 1)
        out.append((n - last) * math.exp(-i / RATE * 90) * gain)
        last = n
    return out


def pluck(note, length=0.28, gain=1.0):
    f, out, low = midi(note), [], 0.0
    for i in range(int(RATE * length)):
        t = i / RATE
        saw = 2 * (t * f % 1) - 1
        cut = 0.03 + 0.5 * math.exp(-t * 16)
        low += (saw - low) * cut
        out.append(low * math.exp(-t * 7) * gain)
    return out


def pad(notes, length, gain=1.0):
    n = int(RATE * length)
    out = [0.0] * n
    for note in notes:
        for detune in (-0.08, 0.0, 0.08):
            f = midi(note + detune)
            for i in range(0, n):
                out[i] += math.sin(2 * math.pi * f * i / RATE + detune * 9)
    fade = RATE // 3
    for i in range(n):
        env = min(1.0, i / fade, (n - i) / fade)
        out[i] *= env * gain / (len(notes) * 3)
    return out


def riser(length, gain=1.0):
    n = int(RATE * length)
    out, low = [], 0.0
    for i in range(n):
        p = i / n
        low += (rng.uniform(-1, 1) - low) * (0.02 + 0.6 * p * p)
        out.append(low * p * p * gain)
    return out


def boom(gain=1.0):
    out, phase, low = [], 0.0, 0.0
    for i in range(int(RATE * 1.6)):
        t = i / RATE
        phase += 2 * math.pi * (32 + 90 * math.exp(-t * 16)) / RATE
        low += (rng.uniform(-1, 1) - low) * max(0.01, 0.4 * math.exp(-t * 5))
        out.append((math.sin(phase) * math.exp(-t * 2.6) + low * math.exp(-t * 3)) * gain)
    return out


# Bars of four beats, each with its chord: i VI III VII, in A minor.
CHORDS = [[57, 60, 64, 71], [53, 57, 60, 67], [48, 55, 60, 64], [55, 59, 62, 69]]
BARS = SECONDS // 2
for bar in range(BARS):
    chord = CHORDS[bar % 4]
    at = bar * BEAT * 4
    add(at, pad(chord, 2.1), 0.5, 0.17)
    # In with the first act's end; out for the last bars.
    if bar >= 2 and bar < BARS - 1:
        for beat in range(4):
            add(at + beat * BEAT, kick(), 0.5, 0.62)
            add(at + beat * BEAT + BEAT // 2, hat(), 0.65, 0.14)
        for step in range(8):
            add(at + step * BEAT // 2, pluck(chord[0] - 24 + (12 if step % 3 == 2 else 0), 0.3), 0.5, 0.26)
    if bar >= 4 and bar < BARS - 1:
        for step in range(16):
            note = chord[(step * 3 + bar) % 4] + 12 * (1 + step % 2)
            add(at + step * BEAT // 4, pluck(note, 0.22), 0.25 + 0.5 * (step % 4) / 3, 0.075)

# A hit on each cut (frames of the film), a rise into it.
for frame in (240, 480, 840, 1080, 1320, 1680):
    at = frame * RATE // FPS
    add(max(0, at - RATE), riser(1.0), 0.5, 0.13)
    add(at, boom(), 0.5, 0.42)
# The first spark's pop, and the logo's landing.
add(90 * RATE // FPS, boom(), 0.5, 0.38)
add(1770 * RATE // FPS, boom(), 0.5, 0.5)

peak = max(max(abs(s) for s in left), max(abs(s) for s in right)) or 1.0
out = array.array("h", [0]) * (FRAMES * 2)
for i in range(FRAMES):
    fade = min(1.0, (FRAMES - i) / RATE)
    out[i * 2] = int(left[i] / peak * fade * 30000)
    out[i * 2 + 1] = int(right[i] / peak * fade * 30000)
path = pathlib.Path(__file__).with_name("score.wav")
with wave.open(str(path), "wb") as file:
    file.setnchannels(2)
    file.setsampwidth(2)
    file.setframerate(RATE)
    file.writeframes(out.tobytes())
print(path)
