"""Generate synthetic silence used by HLS decoder regression tests.

PyAV is only needed to regenerate these fixtures, never for the player or tests.
"""

import argparse
from fractions import Fraction
import hashlib
import io
from pathlib import Path
import struct
import sys


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pyav-path", type=Path)
    args = parser.parse_args()
    if args.pyav_path:
        sys.path.insert(0, str(args.pyav_path.resolve()))
    import av

    output_directory = Path(__file__).resolve().parent

    def encode(container_format, options=None):
        buffer = io.BytesIO()
        with av.open(buffer, "w", format=container_format, options=options or {}) as container:
            stream = container.add_stream("aac", rate=48000)
            stream.layout = "mono"
            stream.bit_rate = 64000
            for index in range(8):
                frame = av.AudioFrame(format="fltp", layout="mono", samples=1024)
                frame.sample_rate = 48000
                frame.time_base = Fraction(1, 48000)
                frame.pts = index * 1024
                for plane in frame.planes:
                    plane.update(bytes(plane.buffer_size))
                for packet in stream.encode(frame):
                    container.mux(packet)
            for packet in stream.encode(None):
                container.mux(packet)
        return buffer.getvalue()

    adts = encode("adts")
    fragmented_mp4 = encode("mp4", {"movflags": "frag_keyframe+empty_moov+default_base_moof"})
    transport_stream = encode("mpegts")
    initialization = bytearray()
    fragment = bytearray()
    offset = 0
    while offset < len(fragmented_mp4):
        size, kind = struct.unpack_from(">I4s", fragmented_mp4, offset)
        if size < 8 or offset + size > len(fragmented_mp4):
            raise ValueError("Unexpected MP4 box boundary")
        data = fragmented_mp4[offset:offset + size]
        if kind in (b"ftyp", b"moov"):
            initialization.extend(data)
        elif kind in (b"moof", b"mdat"):
            fragment.extend(data)
        elif kind != b"mfra":
            raise ValueError(f"Unexpected MP4 box {kind!r}")
        offset += size
    assert initialization and fragment
    files = {
        "hls-silence.aac": adts,
        "hls-init.mp4": bytes(initialization),
        "hls-segment.m4s": bytes(fragment),
        "hls-silence.ts": transport_stream,
    }
    for name, data in files.items():
        (output_directory / name).write_bytes(data)
        print(name, len(data), hashlib.sha256(data).hexdigest())

    for name, data, container_format in [
        ("ADTS", adts, "aac"),
        ("fMP4", bytes(initialization + fragment), "mp4"),
        ("TS", transport_stream, "mpegts"),
    ]:
        samples = 0
        peak = 0.0
        with av.open(io.BytesIO(data), format=container_format) as container:
            stream = container.streams.audio[0]
            assert stream.codec_context.name == "aac"
            assert stream.codec_context.sample_rate == 48000
            assert stream.codec_context.layout.name == "mono"
            for frame in container.decode(audio=0):
                assert frame.format.name == "fltp"
                samples += frame.samples
                values = memoryview(frame.planes[0]).cast("f")[:frame.samples]
                peak = max(peak, max(abs(value) for value in values))
        assert samples > 0 and peak < 0.000001
        print(name, "decoded", samples, "PCM samples; peak", peak)
    print("PyAV", av.__version__, "FFmpeg libraries", av.library_versions)


if __name__ == "__main__":
    main()
