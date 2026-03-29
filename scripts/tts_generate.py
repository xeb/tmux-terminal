# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "pocket-tts",
#     "scipy",
#     "numpy",
# ]
# ///

import argparse

import numpy as np
import scipy.io.wavfile
from pocket_tts import TTSModel


CATALOG_VOICES = ["alba", "marius", "javert", "jean", "fantine", "cosette", "eponine", "azelma"]


def generate_wav(text: str, output: str, voice: str = "alba") -> None:
    """Generate WAV audio using Pocket TTS."""
    tts_model = TTSModel.load_model()
    voice_state = tts_model.get_state_for_audio_prompt(voice)
    audio = tts_model.generate_audio(voice_state, text)
    # Convert float32 to PCM 16-bit for maximum browser compatibility
    samples = audio.numpy()
    samples = np.clip(samples, -1.0, 1.0)
    pcm16 = (samples * 32767).astype(np.int16)
    scipy.io.wavfile.write(output, tts_model.sample_rate, pcm16)


def main():
    parser = argparse.ArgumentParser(description="Generate TTS audio using Pocket TTS")
    parser.add_argument("--text", required=True, help="Text to synthesize")
    parser.add_argument("--output", required=True, help="Output WAV file path")
    parser.add_argument("--voice", default="alba", help="Voice name: alba, marius, javert, jean, fantine, cosette, eponine, azelma")
    args = parser.parse_args()

    generate_wav(args.text, args.output, args.voice)


if __name__ == "__main__":
    main()
