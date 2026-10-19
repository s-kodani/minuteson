import {test} from "node:test";
import assert from "node:assert/strict";
import {matchingThreshold,defaultVoiceMatchingPreferences} from "../src/voice-matching-state.ts";
test("voice thresholds accept endpoints and hundredths only",()=>{
 assert.equal(matchingThreshold("0.80"),800);assert.equal(matchingThreshold("0"),0);assert.equal(matchingThreshold("1.00"),1000);
 for(const input of [""," ","NaN","Infinity","-0.01","1.01","0.801"]){assert.throws(()=>matchingThreshold(input));}
 assert.deepEqual(defaultVoiceMatchingPreferences,{similarityThresholdMilli:800,marginThresholdMilli:100});
});
