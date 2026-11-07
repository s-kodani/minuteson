import test from "node:test";
import assert from "node:assert/strict";
import {nativeAbsolutePath,displayNativePath,displayNativePathsInText} from "../src/native-path.ts";
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

test("diagnostics remove drive and UNC prefixes across paths and lines without changing surrounding text",()=>{
  const drive=String.raw`C:\利用者\会議 データ`,unc=String.raw`\\server\共有\会議`,prefix=String.raw`\\?`+"\\";
  const raw=`検証失敗: ${prefix}${drive}\n退避先: ${prefix}UNC\\${unc.slice(2)} と ${prefix}${drive}\\${"長い名前".repeat(300)}`;
  assert.equal(displayNativePathsInText(raw),`検証失敗: ${drive}\n退避先: ${unc} と ${drive}\\${"長い名前".repeat(300)}`);
  for(const value of [drive,unc,"/Users/利用者/会議",String.raw`\\?\Volume{123}\file`,String.raw`\\.\C:\device`,String.raw`\\?\GLOBALROOT\Device\HarddiskVolume1`,String.raw`\\?\C:relative`])assert.equal(displayNativePathsInText(value),value);
});

test("diagnostics preserve Rust Debug and nested JSON backslash escaping and all non-prefix characters",()=>{
  const raw=String.raw`\\?\C:\利用者\会議 データ`,unc=String.raw`\\?\UNC\server\共有\会議`;
  const plain=String.raw`C:\利用者\会議 データ`,plainUnc=String.raw`\\server\共有\会議`;
  for(let escapes=0;escapes<4;escapes++){
    const escape=(value:string)=>{for(let n=0;n<escapes;n++)value=JSON.stringify(value);return value;};
    assert.equal(displayNativePathsInText(`path=${escape(raw)}; unc=${escape(unc)}`),`path=${escape(plain)}; unc=${escape(plainUnc)}`);
  }
});
