#!/usr/bin/env python3
import hashlib
import json
import zlib
from image_resource_color_v1 import derive_image_color_disposition_v1
from test_image_resource_color_v1 import chunk, icc_profile, jpeg_segment, jpeg_with, png_with, exif_colorspace

def receipt(data, mime, name):
    sha = hashlib.sha256(data).hexdigest()
    d = derive_image_color_disposition_v1(
        asset_bytes=data, mime_type=mime, expected_sha256=sha, resource_id="fixture:" + name
    )
    return {
        "name": name,
        "resource_id": d.resource_id,
        "content_hash": d.content_sha256,
        "mime": d.mime_type,
        "parser_version": d.parser_version,
        "disposition_class": d.disposition_class,
        "exact_color_ready": d.exact_color_ready,
        "metadata_sources": d.metadata_sources,
        "reason_codes": d.reason_codes,
        "icc": None if d.icc is None else {
            "sha256": d.icc.sha256,
            "byte_len": d.icc.byte_len,
            "major_version": d.icc.major_version,
            "profile_class": d.icc.profile_class,
            "data_color_space": d.icc.data_color_space,
            "pcs": d.icc.pcs,
        },
    }

profile = icc_profile()
fixtures = [
    receipt(png_with(color_chunks=[chunk(b"sRGB", b"\x00")]), "image/png", "png-srgb"),
    receipt(png_with(color_chunks=[chunk(b"iCCP", b"profile\x00\x00" + zlib.compress(profile))]), "image/png", "png-icc"),
    receipt(jpeg_with(app_segments=[jpeg_segment(0xE1, exif_colorspace(1))]), "image/jpeg", "jpeg-exif-srgb"),
    receipt(jpeg_with(), "image/jpeg", "jpeg-untagged"),
]
print(json.dumps({
    "schema": "chaptera.image-resource-color-receipt.v1",
    "measurement_class": "synthetic_exact_image_metadata",
    "real_pub": False,
    "representative": False,
    "fixtures": fixtures,
    "raw_profile_bytes_emitted": False,
    "pixel_conversion_performed": False,
}, indent=2, sort_keys=True))
