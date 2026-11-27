#include <cstdio>
#include <string>
#include "leveldb/db.h"
#include "leveldb/options.h"
static std::string key(int i){char b[32];snprintf(b,sizeof b,"k%06d",i);return b;}
static std::string value(int i){return "valeur-"+std::to_string(i)+"-"+std::string(i%300,'x');}
int main(int argc,char**argv){
  std::string mode=argv[1], dir=argv[2];
  leveldb::Options o; o.paranoid_checks=true; o.compression=leveldb::kNoCompression;
  leveldb::DB* db;
  if(mode=="check"||mode=="check2"){
    auto s=leveldb::DB::Open(o,dir,&db); if(!s.ok()){printf("OPEN FAIL %s\n",s.ToString().c_str());return 1;}
    leveldb::ReadOptions r; r.verify_checksums=true; int n=0,bad=0; auto it=db->NewIterator(r);
    for(it->SeekToFirst();it->Valid();it->Next()){n++;
      std::string k=it->key().ToString();
      if(k=="basalt"){ if(it->value().ToString()!="etait ici") bad++; continue;}
      int i=atoi(k.c_str()+1); if(it->value().ToString()!=value(i)) bad++; }
    if(!it->status().ok()){printf("ITER FAIL %s\n",it->status().ToString().c_str());return 1;}
    delete it;
    std::string v; int miss=0;
    if(mode=="check"){ for(int i=0;i<5000;i++){auto g=db->Get(r,key(i),&v); bool gone=(i%100==0&&i<5000); if(g.ok()==gone) miss++;} }
    else { auto g=db->Get(r,"basalt",&v); if(!g.ok()) miss++; g=db->Get(r,key(1),&v); if(g.ok()) miss++; g=db->Get(r,key(2),&v); if(!g.ok()) miss++; }
    printf("entries %d bad %d wrong-gets %d\n",n,bad,miss);
    db->CompactRange(nullptr,nullptr); delete db; return bad||miss;
  }
  if(mode=="make"){
    o.create_if_missing=true; o.write_buffer_size=256*1024;
    auto s=leveldb::DB::Open(o,dir,&db); if(!s.ok()){printf("OPEN FAIL\n");return 1;}
    leveldb::WriteOptions w;
    for(int i=0;i<20000;i++) db->Put(w,key(i),"old");
    db->CompactRange(nullptr,nullptr);
    for(int i=0;i<20000;i++) db->Put(w,key(i),value(i));
    for(int i=0;i<20000;i+=7) if(i<15000) db->Delete(w,key(i));
    delete db;
    // reouverture : les dernieres suppressions restent dans le journal
    s=leveldb::DB::Open(o,dir,&db); for(int i=14994;i<20000;i++) if(i%7==0) db->Delete(w,key(i)); delete db; return 0;
  }
}
