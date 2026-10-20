// Called by the Rust controller test with real snapshots from private fixtures.
import { readFileSync } from "node:fs";
import { assertDataSnapshot } from "../src/data-operation-state.ts";
import { assertModelDeletionPage } from "../src/model-deletion-operation-state.ts";

for (const { snapshot, page } of JSON.parse(readFileSync(0, "utf8"))) {
  try {
    assertDataSnapshot(snapshot);
    assertModelDeletionPage(page, snapshot.modelDeletionPreview, 0);
  } catch (cause) {
    throw new Error(`Native model deletion contract rejected ${JSON.stringify(snapshot)}`, { cause });
  }
}
