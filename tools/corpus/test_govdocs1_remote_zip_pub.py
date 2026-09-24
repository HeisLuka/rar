#!/usr/bin/env python3
import binascii
import io
import struct
import sys
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import govdocs1_remote_zip_pub as g


class RemoteZipTests(unittest.TestCase):
    def test_safe_member(self):
        self.assertTrue(g.safe_member("000/000001.pub"))
        self.assertFalse(g.safe_member("../evil.pub"))
        self.assertFalse(g.safe_member("/abs.pub"))

    def test_parse_standard_zip_central_directory(self):
        buf = io.BytesIO()
        with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_DEFLATED) as zf:
            zf.writestr("a.pub", b"hello")
            zf.writestr("b.txt", b"world")
        data = buf.getvalue()
        tail = data[-min(len(data), g.MAX_TAIL):]
        tail_start = len(data) - len(tail)
        count, cd_offset, cd_size = g.parse_eocd(tail, tail_start)
        entries = g.parse_central_directory(data[cd_offset:cd_offset+cd_size], count)
        self.assertEqual([e.name for e in entries], ["a.pub", "b.txt"])

    def test_decode_deflate_member(self):
        raw = b"publisher-bytes" * 100
        comp = zlib.compress(raw)[2:-4]
        entry = g.Entry(
            name="x.pub", flags=0, method=8,
            crc32=binascii.crc32(raw) & 0xffffffff,
            compressed_size=len(comp), uncompressed_size=len(raw), local_offset=0,
        )
        self.assertEqual(g.decode_member(entry, comp, 1024*1024), raw)


if __name__ == "__main__":
    import zlib
    unittest.main()
