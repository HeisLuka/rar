import copy, importlib.util, json, pathlib, unittest
ROOT=pathlib.Path(__file__).resolve().parents[1]
VP=ROOT/"tools"/"validate_pub_research_receipt.py"
FP=ROOT/"packages"/"research"/"pub-research-receipt"/"v1"/"fixtures"/"synthetic.json"

def load_validator():
    spec=importlib.util.spec_from_file_location("pub_research_receipt_validator",VP)
    mod=importlib.util.module_from_spec(spec); spec.loader.exec_module(mod); return mod

class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.v=load_validator(); self.base=json.loads(FP.read_text(encoding="utf-8"))
    def test_valid(self): self.assertEqual(self.v.validate(copy.deepcopy(self.base))["task_id"],"PUB-T-746")
    def test_bad_hash(self):
        d=copy.deepcopy(self.base); d["inputs"][0]["sha256"]="x"
        with self.assertRaises(SystemExit): self.v.validate(d)
    def test_local_path_rejected(self):
        d=copy.deepcopy(self.base); d["limitations"][0]="D:"+chr(92)+"Downloads"+chr(92)+"secret.pub"
        with self.assertRaises(SystemExit): self.v.validate(d)
    def test_secret_rejected(self):
        d=copy.deepcopy(self.base); d["scope"]="token "+"ghp_"+("A"*40)
        with self.assertRaises(SystemExit): self.v.validate(d)
    def test_privacy_true_rejected(self):
        d=copy.deepcopy(self.base); d["privacy"]["local_path_in_receipt"]=True
        with self.assertRaises(SystemExit): self.v.validate(d)
    def test_count_overclaim_rejected(self):
        d=copy.deepcopy(self.base); d["counts"]={"observations":1,"supporting":1,"counterexamples":1}
        with self.assertRaises(SystemExit): self.v.validate(d)
    def test_extra_field_rejected(self):
        d=copy.deepcopy(self.base); d["extra"]="no"
        with self.assertRaises(SystemExit): self.v.validate(d)

if __name__=="__main__": unittest.main()
