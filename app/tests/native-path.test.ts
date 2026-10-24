import test from "node:test";
import assert from "node:assert/strict";
import {nativeAbsolutePath,displayNativePath} from "../src/native-path.ts";
import {checkedStartupReport} from "../src/startup-attention-state.ts";
test("Windows and macOS native paths retain their absolute spelling",()=>{
  for(const path of ["C:\\会議 データ\\mon", "C:/会議/mon", "\\\\?\\C:\\会議\\mon", "/Users/example/会議"]){
    assert.ok(nativeAbsolutePath(path));
    const report=checkedStartupReport({schemaVersion:1,platform:"windows",phase:"ready",failure:null,recovery:{kind:"copy_restored",reviewRequired:false,reasons:[],recordPath:path}});
    assert.equal(report.recovery?.recordPath,path);
  }
  for(const path of ["relative", "C:relative", "\\root-relative", "C:\\bad\0path"]){assert.equal(nativeAbsolutePath(path),false);}
});

test("display paths remove only recognized Windows extended prefixes",()=>{
  const b=String.fromCharCode(92),normal=["C:","会議 データ","mon"].join(b),unc=b.repeat(2)+["server","share","会議"].join(b);
  assert.equal(displayNativePath(b.repeat(2)+"?"+b+normal),normal);
  assert.equal(displayNativePath(b.repeat(2)+"?"+b+"UNC"+b+unc.slice(2)),unc);
  for(const path of [normal,unc,"/Users/example/会議","C:/会議",b.repeat(2)+"?"+b+"Volume{123}"+b])assert.equal(displayNativePath(path),path);
  assert.equal(displayNativePath(null),"");
});
