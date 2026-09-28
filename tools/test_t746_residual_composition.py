import csv
import importlib.util
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "pub-research" / "verify_t746_residual_composition.py"

def load_tool():
    spec = importlib.util.spec_from_file_location("t746_tool", TOOL)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod

def write_csv(path, fieldnames, rows):
    with path.open("w", encoding="utf-8", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=fieldnames)
        w.writeheader()
        w.writerows(rows)

class T746CompositionTests(unittest.TestCase):
    def setUp(self):
        self.tool = load_tool()
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def make_positive(self):
        residual = self.root / "residual.csv"
        raw29 = self.root / "raw29.csv"
        residual_rows = []
        raw29_rows = []
        for i in range(171):
            residual_rows.append({"raw_type":"0x29","source_sha256":f"{i:064x}","object_id":str(i),"offset":str(i*10)})
            raw29_rows.append({"raw_type":"0x29","source_sha256":f"{i:064x}","object_id":str(i),"offset":str(i*10),"has_direct_0x36":"0"})
        for i in range(171,177):
            raw29_rows.append({"raw_type":"0x29","source_sha256":f"{i:064x}","object_id":str(i),"offset":str(i*10),"has_direct_0x36":"1"})
        for i in range(5):
            residual_rows.append({"raw_type":"0x1e","source_sha256":f"{1000+i:064x}","object_id":str(1000+i),"offset":str(2000+i)})
        write_csv(residual,["raw_type","source_sha256","object_id","offset"],residual_rows)
        write_csv(raw29,["raw_type","source_sha256","object_id","offset","has_direct_0x36"],raw29_rows)
        return residual, raw29

    def test_exact_hypothesis_confirms(self):
        residual, raw29 = self.make_positive()
        receipt = self.tool.analyse(residual, raw29)
        self.assertEqual(receipt["verdict"], "confirmed")
        self.assertEqual(receipt["counts"]["marker_counts"], {"0x1e":5,"0x29":171})
        self.assertEqual(receipt["counts"]["raw29_reference_leaf"], 171)
        self.assertEqual(receipt["counts"]["raw29_reference_nested"], 6)

    def test_other_marker_refutes(self):
        residual, raw29 = self.make_positive()
        rows, fields = self.tool.load_rows(residual)
        rows[-1]["raw_type"] = "0x14"
        write_csv(residual, fields, rows)
        receipt = self.tool.analyse(residual, raw29)
        self.assertEqual(receipt["verdict"], "refuted")
        self.assertEqual(receipt["counts"]["other_marker_rows"], 1)

    def test_missing_raw29_join_is_indeterminate_when_counts_match(self):
        residual, raw29 = self.make_positive()
        rows, fields = self.tool.load_rows(raw29)
        rows[0]["object_id"] = "999999"
        write_csv(raw29, fields, rows)
        receipt = self.tool.analyse(residual, raw29)
        self.assertEqual(receipt["verdict"], "indeterminate")
        self.assertEqual(receipt["counts"]["raw29_residual_missing_from_reference"], 1)

    def test_marker_autodetection_accepts_decimal(self):
        residual, raw29 = self.make_positive()
        rows, fields = self.tool.load_rows(residual)
        for row in rows:
            row["raw_type"] = str(int(row["raw_type"], 16))
        write_csv(residual, fields, rows)
        receipt = self.tool.analyse(residual, raw29)
        self.assertEqual(receipt["verdict"], "confirmed")

    def test_receipt_has_no_row_identity_dump(self):
        residual, raw29 = self.make_positive()
        receipt = self.tool.analyse(residual, raw29)
        serialized = json.dumps(receipt)
        self.assertNotIn("0000000000000000000000000000000000000000000000000000000000000001", serialized)
        self.assertFalse(receipt["privacy"]["row_identities_emitted"])

if __name__ == "__main__":
    unittest.main()
