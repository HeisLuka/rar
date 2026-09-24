#!/usr/bin/env python3
from __future__ import annotations
import argparse,hashlib,io,json,struct
from collections import Counter
from pathlib import Path
SIG=bytes.fromhex('d0cf11e0a1b11ae1'); FREE=0xffffffff; END=0xfffffffe; FAT=0xfffffffd; DIF=0xfffffffc; NO=0xffffffff
SPECIAL={FREE,END,FAT,DIF}; SCHEMA='chaptera.cfb-physical-diff.v1'
u16=lambda b,o:struct.unpack_from('<H',b,o)[0]; u32=lambda b,o:struct.unpack_from('<I',b,o)[0]; u64=lambda b,o:struct.unpack_from('<Q',b,o)[0]
def h(b):return hashlib.sha256(b).hexdigest()
def name(raw,n):
    return raw[:n-2].decode('utf-16le',errors='replace') if 2<=n<=64 and n%2==0 else ''
class CFB:
 def __init__(self,b):
  self.b=b
  if len(b)<512 or b[:8]!=SIG:raise ValueError('not CFB')
  self.major=u16(b,26); self.ss=1<<u16(b,30); self.ms=1<<u16(b,32)
  if self.ss not in (512,4096) or self.ms!=64 or len(b)%self.ss:raise ValueError('unsupported/alignment')
  self.nsec=len(b)//self.ss-1; self.nfat=u32(b,44); self.dir0=u32(b,48); self.cut=u32(b,56); self.mini0=u32(b,60); self.nmini=u32(b,64); self.dif0=u32(b,68); self.ndif=u32(b,72)
  self.difsecs=[]; self.fatsecs=self._difat(); self.fat=self._table(self.fatsecs); self.dirsecs=self.chain(self.dir0,self.fat,'dir'); self.minisecs=self.chain(self.mini0,self.fat,'minifat') if self.nmini else []
  if len(self.minisecs)!=self.nmini:raise ValueError('MiniFAT count')
  self.minifat=self._table(self.minisecs); self.dirs=self._dirs(); roots=[x for x in self.dirs if x['type']==5]
  if not roots:raise ValueError('no root')
  self.root=roots[0]; self.rootchain=self.chain(self.root['start'],self.fat,'root') if self.root['size'] else []
  self.lab=['unknown']*len(b); self._labels()
 def off(self,s):
  if s<0 or s>=self.nsec:raise ValueError(f'sector range {s}')
  return (s+1)*self.ss
 def sec(self,s):o=self.off(s);return self.b[o:o+self.ss]
 def _difat(self):
  ids=[u32(self.b,76+4*i) for i in range(109)];ids=[x for x in ids if x!=FREE];s=self.dif0;seen=set();n=self.ss//4-1
  for _ in range(self.ndif):
   if s in SPECIAL or s in seen:raise ValueError('DIFAT chain')
   seen.add(s);self.difsecs.append(s);q=self.sec(s);ids += [u32(q,4*i) for i in range(n) if u32(q,4*i)!=FREE];s=u32(q,self.ss-4)
  if len(ids)<self.nfat:raise ValueError('FAT ids')
  return ids[:self.nfat]
 def _table(self,sectors):
  out=[]
  for s in sectors:out.extend(struct.unpack('<'+'I'*(self.ss//4),self.sec(s)))
  return out
 def chain(self,s,t,label):
  if s in (FREE,END):return []
  out=[];seen=set();bound=self.nsec if t is self.fat else len(t)
  while s!=END:
   if s in SPECIAL or s in seen or s>=bound or s>=len(t):raise ValueError(label+' chain')
   seen.add(s);out.append(s);s=t[s]
   if len(out)>bound+1:raise ValueError(label+' long')
  return out
 def _dirs(self):
  out=[];idx=0
  for s in self.dirsecs:
   q=self.sec(s);base=self.off(s)
   for p in range(0,self.ss,128):
    r=q[p:p+128];typ=r[66]
    if typ not in (0,1,2,5):raise ValueError('dir type')
    out.append({'i':idx,'name':name(r[:64],u16(r,64)),'type':typ,'color':r[67],'left':u32(r,68),'right':u32(r,72),'child':u32(r,76),'clsid':r[80:96].hex(),'state':u32(r,96),'ctime':u64(r,100),'mtime':u64(r,108),'start':u32(r,116),'size':u64(r,120),'raw':base+p});idx+=1
  return out
 def schain(self,e):
  if not e['size']:return []
  return self.chain(e['start'],self.minifat if e['type']==2 and e['size']<self.cut else self.fat,'stream '+e['name'])
 def mark(self,a,z,x):
  for i in range(max(0,a),min(len(self.lab),z)):self.lab[i]=x
 def msec(self,s,x):o=self.off(s);self.mark(o,o+self.ss,x)
 def rawmini(self,o):
  n,r=divmod(o,self.ss)
  if n>=len(self.rootchain):raise ValueError('mini root range')
  return self.off(self.rootchain[n])+r
 def _labels(self):
  self.mark(0,self.ss,'header_area')
  for a,z,x in [(0,8,'signature'),(8,24,'clsid'),(24,26,'minor'),(26,28,'major'),(28,30,'byte_order'),(30,32,'sector_shift'),(32,34,'mini_shift'),(34,40,'reserved'),(40,44,'num_dir'),(44,48,'num_fat'),(48,52,'first_dir'),(52,56,'transaction'),(56,60,'mini_cutoff'),(60,64,'first_minifat'),(64,68,'num_minifat'),(68,72,'first_difat'),(72,76,'num_difat'),(76,512,'difat_slots')]:self.mark(a,z,'header.'+x)
  for s in range(self.nsec):self.msec(s,'unallocated_sector' if (s>=len(self.fat) or self.fat[s]==FREE) else 'allocated_other_sector')
  for s in self.difsecs:self.msec(s,'difat_sector')
  for s in self.fatsecs:self.msec(s,'fat_sector')
  for s in self.minisecs:self.msec(s,'minifat_sector')
  for s in self.dirsecs:self.msec(s,'directory_sector')
  fields=[(0,64,'name'),(64,66,'name_length'),(66,67,'type'),(67,68,'color'),(68,72,'left'),(72,76,'right'),(76,80,'child'),(80,96,'clsid'),(96,100,'state'),(100,108,'ctime'),(108,116,'mtime'),(116,120,'start'),(120,128,'size')]
  for e in self.dirs:
   for a,z,x in fields:self.mark(e['raw']+a,e['raw']+z,f"directory[{e['i']}].{x}")
  for s in self.rootchain:self.msec(s,'root_ministream_container')
  for e in self.dirs:
   if e['type']!=2 or not e['size']:continue
   rem=e['size']
   if e['size']>=self.cut:
    for s in self.schain(e):
     o=self.off(s);n=min(rem,self.ss);self.mark(o,o+n,'stream_payload:'+e['name']);self.mark(o+n,o+self.ss,'stream_slack:'+e['name']);rem-=n
   else:
    for s in self.schain(e):
     o=self.rawmini(s*self.ms);n=min(rem,self.ms);self.mark(o,o+n,'mini_stream_payload:'+e['name']);self.mark(o+n,o+self.ms,'mini_stream_slack:'+e['name']);rem-=n
   if rem:raise ValueError('stream chain too short')
 def summary(self):
  streams=[]
  for e in self.dirs:
   if e['type']==2 and e['size']:streams.append({'i':e['i'],'name':e['name'],'storage':'minifat' if e['size']<self.cut else 'fat','chain':self.schain(e),'size':e['size']})
  return {'sha256':h(self.b),'byte_len':len(self.b),'major':self.major,'sector_size':self.ss,'mini_sector_size':self.ms,'fat_sector_ids':self.fatsecs,'difat_sector_ids':self.difsecs,'directory_sector_ids':self.dirsecs,'minifat_sector_ids':self.minisecs,'root_chain':self.rootchain,'streams':streams,'directory_entries':[{k:v for k,v in e.items() if k!='raw'} for e in self.dirs if e['type']]}
def logical(b):
 import olefile
 out={}
 with olefile.OleFileIO(io.BytesIO(b)) as o:
  for p in o.listdir(streams=True,storages=False):
   q=o.openstream(p).read();out['/'+'/'.join(p)]={'len':len(q),'sha256':h(q)}
 return dict(sorted(out.items()))
def broad(x):
 if x.startswith('header.'):return 'header'
 if x.startswith('directory[') or x=='directory_sector':return 'directory'
 if x in ('fat_sector','difat_sector','minifat_sector'):return x.split('_')[0]
 if 'stream_payload:' in x:return 'stream_payload'
 if 'stream_slack:' in x:return 'stream_slack'
 if x=='root_ministream_container':return x
 if x=='unallocated_sector':return 'unallocated'
 if x=='allocated_other_sector':return 'allocated_other'
 return 'unclassified'
def ranges(offs,labs):
 if not offs:return []
 out=[];a=p=offs[0];x=labs[0]
 for o,y in zip(offs[1:],labs[1:]):
  if o==p+1 and y==x:p=o;continue
  out.append({'offset':a,'length':p-a+1,'category':x});a=p=o;x=y
 out.append({'offset':a,'length':p-a+1,'category':x});return out
def tablediff(a,b,n):return [{'index':i,'left':a[i] if i<len(a) else None,'right':b[i] if i<len(b) else None} for i in range(n) if (a[i] if i<len(a) else None)!=(b[i] if i<len(b) else None)]
def chainmap(m):return {(e['i'],e['name']):{'storage':'minifat' if e['size']<m.cut else 'fat','chain':m.schain(e),'size':e['size']} for e in m.dirs if e['type']==2 and e['size']}
def compare(a,b,n=''):
 if len(a)!=len(b) or h(a)==h(b):raise ValueError('pair identity/length invariant')
 if logical(a)!=logical(b):raise ValueError('logical streams differ')
 l=CFB(a);r=CFB(b);off=[i for i,(x,y) in enumerate(zip(a,b)) if x!=y];exact=[];wide=[];unc=0
 for i in off:
  x,y=l.lab[i],r.lab[i];e=x if x==y else x+'->'+y;u,v=broad(x),broad(y);w=u if u==v else u+'->'+v;exact.append(e);wide.append(w);unc+=('unclassified' in w)
 ld={e['i']:{k:v for k,v in e.items() if k!='raw'} for e in l.dirs if e['type']};rd={e['i']:{k:v for k,v in e.items() if k!='raw'} for e in r.dirs if e['type']};dd=[]
 for i in sorted(set(ld)|set(rd)):
  x,y=ld.get(i,{}),rd.get(i,{});c={k:{'left':x.get(k),'right':y.get(k)} for k in sorted(set(x)|set(y)) if x.get(k)!=y.get(k)}
  if c:dd.append({'index':i,'changed_fields':c})
 lc,rc=chainmap(l),chainmap(r);cd=[]
 for k in sorted(set(lc)|set(rc),key=lambda q:(q[0],q[1].casefold(),q[1])):
  if lc.get(k)!=rc.get(k):cd.append({'directory_index':k[0],'name':k[1],'left':lc.get(k),'right':rc.get(k)})
 return {'schema':SCHEMA,'name':n,'left_sha256':h(a),'right_sha256':h(b),'byte_len':len(a),'logical_stream_count':len(logical(a)),'logical_streams_identical':True,'different_byte_count':len(off),'classified_byte_count':len(off)-unc,'unclassified_byte_count':unc,'broad_category_counts':dict(sorted(Counter(wide).items())),'exact_category_counts':dict(sorted(Counter(exact).items())),'diff_ranges':ranges(off,exact),'left_physical':l.summary(),'right_physical':r.summary(),'directory_metadata_diffs':dd,'fat_table_diff_entries':tablediff(l.fat,r.fat,max(l.nsec,r.nsec)),'minifat_table_diff_entries':tablediff(l.minifat,r.minifat,max(len(l.minifat),len(r.minifat))),'stream_chain_diffs':cd,'fat_sector_ids_equal':l.fatsecs==r.fatsecs,'difat_sector_ids_equal':l.difsecs==r.difsecs,'directory_sector_ids_equal':l.dirsecs==r.dirsecs,'minifat_sector_ids_equal':l.minisecs==r.minisecs,'root_chain_equal':l.rootchain==r.rootchain}
def minimal(state=0):
 s=512;b=bytearray(s*11);b[:8]=SIG
 for o,v,fmt in [(24,0x3e,'H'),(26,3,'H'),(28,0xfffe,'H'),(30,9,'H'),(32,6,'H'),(44,1,'I'),(48,0,'I'),(56,4096,'I'),(60,END,'I'),(68,END,'I')]:struct.pack_into('<'+fmt,b,o,v)
 for i in range(109):struct.pack_into('<I',b,76+4*i,FREE)
 struct.pack_into('<I',b,76,9)
 def de(o,n,t,ch,st,z,state=0):
  e=(n+'\0').encode('utf-16le');b[o:o+len(e)]=e;struct.pack_into('<H',b,o+64,len(e));b[o+66]=t;b[o+67]=1
  for p,v in [(68,NO),(72,NO),(76,ch),(96,state),(116,st)]:struct.pack_into('<I',b,o+p,v)
  struct.pack_into('<Q',b,o+120,z)
 de(512,'Root Entry',5,1,END,0);de(640,'Data',2,NO,1,4096,state)
 q=bytes(i%251 for i in range(4096))
 for j,sid in enumerate(range(1,9)):b[(sid+1)*512:(sid+2)*512]=q[j*512:(j+1)*512]
 v=[FREE]*128;v[0]=END
 for sid in range(1,8):v[sid]=sid+1
 v[8]=END;v[9]=FAT;struct.pack_into('<'+'I'*128,b,5120,*v);return bytes(b)
def selftest():
 a,b=minimal(0),minimal(1);x,y=CFB(a),CFB(b);d=[i for i,(p,q) in enumerate(zip(a,b)) if p!=q];assert d==[736] and x.lab[d[0]]==y.lab[d[0]]=='directory[1].state'
 fake=CFB.__new__(CFB);fake.nsec=10;fake.fat=[END]*10;mini=[FREE]*32;mini[20]=21;mini[21]=END
 assert fake.chain(20,mini,'mini-regression')==[20,21]
 print('cfb physical diff self-test ok');return 0
def main():
 p=argparse.ArgumentParser();s=p.add_subparsers(dest='cmd',required=True);c=s.add_parser('compare');c.add_argument('--left',type=Path,required=True);c.add_argument('--right',type=Path,required=True);c.add_argument('--name',default='');c.add_argument('--out',type=Path,required=True);s.add_parser('self-test');a=p.parse_args()
 if a.cmd=='self-test':return selftest()
 r=compare(a.left.read_bytes(),a.right.read_bytes(),a.name);a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(r,indent=2,ensure_ascii=False));print(json.dumps({k:r[k] for k in ['name','different_byte_count','unclassified_byte_count','broad_category_counts']},indent=2));return 0
if __name__=='__main__':raise SystemExit(main())
