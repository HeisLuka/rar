#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
import urllib.parse
import urllib.request
from collections import Counter
from pathlib import Path

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import cfb_physical_diff as physical

SECTOR=2048
UA="rar-corpus-physical-range/1.0"

class RangeISO:
    def __init__(self, archive_url: str, expected_root_sha: str):
        self.archive_url=archive_url
        self.expected_root_sha=expected_root_sha
        self.delivery_urls=self._delivery_urls(archive_url)
        self.delivery_url=self._select_delivery()
        self.dir_cache={}

    def _delivery_urls(self,url):
        p=urllib.parse.urlparse(url)
        parts=p.path.split("/")
        if p.netloc!="archive.org" or len(parts)<4 or parts[1]!="download":
            raise ValueError(f"unsupported archive URL: {url}")
        identifier=parts[2]
        filename="/".join(parts[3:])
        req=urllib.request.Request(
            f"https://archive.org/metadata/{identifier}",
            headers={"User-Agent":UA},
        )
        with urllib.request.urlopen(req,timeout=30) as r:
            meta=json.load(r)
        directory=str(meta.get("dir") or "")
        urls=[]
        for key in ("d1","d2"):
            host=str(meta.get(key) or "")
            if host and directory:
                urls.append(
                    "https://"+host+directory.rstrip("/")+"/"+
                    urllib.parse.quote(filename,safe="/")
                )
        urls.extend([url,url+"?download=1"])
        return list(dict.fromkeys(urls))

    def _read_from(self,url,start,length):
        req=urllib.request.Request(
            url,
            headers={
                "User-Agent":UA,
                "Range":f"bytes={start}-{start+length-1}",
                "Accept-Encoding":"identity",
            },
        )
        with urllib.request.urlopen(req,timeout=45) as r:
            status=getattr(r,"status",None)
            content_range=r.headers.get("Content-Range")
            if status!=206 and not content_range:
                raise ValueError(f"range unsupported status={status} url={url}")
            data=r.read(length+1)
        if len(data)!=length:
            raise ValueError(f"range length drift {len(data)} != {length}")
        return data

    def _select_delivery(self):
        last=None
        for url in self.delivery_urls:
            for attempt in range(3):
                try:
                    pvd=self._read_from(url,16*SECTOR,SECTOR)
                    if pvd[0]!=1 or pvd[1:6]!=b"CD001":
                        raise ValueError("not ISO9660 primary volume descriptor")
                    print(f"range delivery selected: {url}")
                    return url
                except Exception as exc:
                    last=exc
                    if attempt<2:
                        time.sleep(2**attempt)
        raise last

    def read(self,start,length):
        last=None
        for attempt in range(3):
            try:
                return self._read_from(self.delivery_url,start,length)
            except Exception as exc:
                last=exc
                if attempt<2:
                    time.sleep(2**attempt)
        raise last

    @staticmethod
    def _record(rec):
        extent=int.from_bytes(rec[2:6],"little")
        size=int.from_bytes(rec[10:14],"little")
        flags=rec[25]
        n=rec[32]
        raw=rec[33:33+n]
        name=raw.decode("ascii",errors="strict")
        return name,extent,size,flags

    def _read_dir(self,extent,size):
        key=(extent,size)
        if key in self.dir_cache:
            return self.dir_cache[key]
        data=self.read(extent*SECTOR,size)
        out={}
        pos=0
        while pos<len(data):
            n=data[pos]
            if n==0:
                pos=((pos//SECTOR)+1)*SECTOR
                continue
            rec=data[pos:pos+n]
            name,ex,sz,flags=self._record(rec)
            if name not in ("\x00","\x01"):
                base=name.split(";")[0].upper()
                out[base]=(ex,sz,flags)
            pos+=n
        self.dir_cache[key]=out
        return out

    def member_bytes(self,path):
        pvd=self.read(16*SECTOR,SECTOR)
        root_len=pvd[156]
        root=pvd[156:156+root_len]
        _,extent,size,flags=self._record(root)
        parts=[x.upper() for x in path.strip("/").split("/") if x]
        for i,part in enumerate(parts):
            listing=self._read_dir(extent,size)
            key=part.split(";")[0]
            if key not in listing:
                raise KeyError(f"ISO member missing: {path} at {part}")
            extent,size,flags=listing[key]
            if i+1<len(parts) and not (flags & 0x02):
                raise ValueError(f"expected directory in path: {part}")
        if flags & 0x02:
            raise ValueError(f"target is directory: {path}")
        return self.read(extent*SECTOR,size)


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--out",type=Path,required=True)
    args=ap.parse_args()
    receipt=json.loads(Path(
        "tools/corpus/receipts/logical-stream-duplicate-locators-2026-09-24.json"
    ).read_text())
    selected=[]
    for group in receipt["groups"]:
        for copy in group["copies"]:
            if copy["root"]=="en":
                selected.append((group["logical_identity"],group["anchor"],copy))
    if len(selected)!=45:
        raise SystemExit(f"expected 45 EN comparisons, got {len(selected)}")

    roots=receipt["roots"]
    readers={
        rid:RangeISO(roots[rid]["url"],roots[rid]["sha256"])
        for rid in ("pub40cd","en")
    }
    results=[]
    cache={}
    def payload(side):
        k=(side["root"],side["member"])
        if k not in cache:
            data=readers[side["root"]].member_bytes(side["member"])
            if len(data)!=side["size"]:
                raise ValueError(f"member size drift {k}: {len(data)} != {side['size']}")
            sha=hashlib.sha256(data).hexdigest()
            if sha!=side["sha256"]:
                raise ValueError(f"member SHA drift {k}: {sha} != {side['sha256']}")
            cache[k]=data
        return cache[k]

    for identity,anchor,copy in selected:
        left=payload(anchor); right=payload(copy)
        r=physical.compare(left,right,f"{anchor['filename']}::en")
        if not r["logical_streams_identical"]:
            raise ValueError(f"logical identity drift: {identity}")
        r.update({
            "logical_identity":identity,
            "anchor_root":"pub40cd",
            "target_root":"en",
            "anchor_member":anchor["member"],
            "target_member":copy["member"],
            "anchor_filename":anchor["filename"],
            "target_filename":copy["filename"],
            "root_identity_mode":"prior-full-root-sha-receipt-plus-current-exact-member-sha",
        })
        sig={
            "broad_categories":sorted(r["broad_category_counts"]),
            "exact_categories":sorted(r["exact_category_counts"]),
            "fat_table_changed":bool(r["fat_table_diff_entries"]),
            "minifat_table_changed":bool(r["minifat_table_diff_entries"]),
            "stream_chain_changed":bool(r["stream_chain_diffs"]),
            "directory_fields":sorted({
                field for row in r["directory_metadata_diffs"]
                for field in row["changed_fields"]
            }),
            "unclassified":r["unclassified_byte_count"],
        }
        r["physical_signature"]=hashlib.sha256(
            json.dumps(sig,sort_keys=True).encode()
        ).hexdigest()
        results.append(r)

    results.sort(key=lambda r:(r["logical_identity"],r["right_sha256"]))
    signatures=Counter(r["physical_signature"] for r in results)
    broad=Counter(); fields=Counter()
    for r in results:
        broad.update(r["broad_category_counts"])
        for row in r["directory_metadata_diffs"]:
            fields.update(row["changed_fields"].keys())
    summary={
        "schema":"chaptera.logical-duplicate-physical-classification-shard.v1",
        "target_root":"en",
        "comparison_count":45,
        "logical_streams_identical":sum(r["logical_streams_identical"] for r in results),
        "fully_classified":sum(r["unclassified_byte_count"]==0 for r in results),
        "with_unclassified":sum(r["unclassified_byte_count"]!=0 for r in results),
        "total_different_bytes":sum(r["different_byte_count"] for r in results),
        "signature_count":len(signatures),
        "signature_sizes":dict(sorted(signatures.items())),
        "broad_category_counts":dict(sorted(broad.items())),
        "directory_changed_fields":dict(sorted(fields.items())),
        "pairs_with_fat_table_diff":sum(bool(r["fat_table_diff_entries"]) for r in results),
        "pairs_with_minifat_table_diff":sum(bool(r["minifat_table_diff_entries"]) for r in results),
        "pairs_with_stream_chain_diff":sum(bool(r["stream_chain_diffs"]) for r in results),
        "rehydration_mode":"ISO9660 HTTP Range; prior root SHA authority + current exact member SHA/size",
        "delivery_urls":{rid:readers[rid].delivery_url for rid in readers},
    }
    args.out.mkdir(parents=True,exist_ok=True)
    (args.out/"pairs.json").write_text(json.dumps(results,indent=2,ensure_ascii=False),encoding="utf-8")
    (args.out/"summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8")
    print(json.dumps(summary,indent=2))
    return 0

if __name__=="__main__":
    raise SystemExit(main())
