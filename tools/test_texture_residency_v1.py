import hashlib, unittest
from texture_residency_v1 import *
def key(data,b="r1",variant="source"): return MaterialKey(b,hashlib.sha256(data).hexdigest(),variant)
class T(unittest.TestCase):
 def test_many_placements_one_upload(self):
  r=TextureResidencyV1(); d=b"x"*1024; k=key(d)
  a=r.demand(k,d); b=r.demand(k,d); self.assertEqual(r.receipt()["metrics"]["uploads"],1); self.assertEqual(r.receipt()["metrics"]["uploads_avoided"],1); self.assertTrue(r.validate_binding(a)); self.assertTrue(r.validate_binding(b))
 def test_variant_does_not_alias(self):
  r=TextureResidencyV1(); d=b"x"*32; r.demand(key(d,variant="thumb"),d); r.demand(key(d,variant="full"),d); self.assertEqual(r.receipt()["metrics"]["uploads"],2)
 def test_resource_id_is_part_of_identity(self):
  r=TextureResidencyV1(); d=b"x"*32; r.demand(key(d,"r1"),d); r.demand(key(d,"r2"),d); self.assertEqual(r.receipt()["unique_materials"],2)
 def test_evict_reload_invalidates_old_binding(self):
  r=TextureResidencyV1(); d=b"abc"; k=key(d); old=r.demand(k,d); r.evict(k); self.assertFalse(r.validate_binding(old)); new=r.demand(k,d); self.assertTrue(r.validate_binding(new)); self.assertEqual(r.receipt()["metrics"]["reloads"],1)
 def test_device_reset_invalidates_every_handle(self):
  r=TextureResidencyV1(); d=b"abc"; k=key(d); old=r.demand(k,d); r.reset_device(); self.assertFalse(r.validate_binding(old)); self.assertEqual(r.receipt()["resident_materials"],0)
 def test_hash_mismatch_fails_closed(self):
  r=TextureResidencyV1(); d=b"abc"; k=key(d); self.assertRaises(ValueError,r.demand,k,b"wrong")
if __name__=="__main__":unittest.main()
