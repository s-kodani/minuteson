// Native operations are synthetic; package contents use tiny fixed metadata.
import React from "react";
import { createRoot } from "react-dom/client";
import { ModelInspectionPanel } from "../src/model-inspection";
import { parseDownloadDefinition, packageReviewHash } from "../src/model-download-state";
import { ToastHost } from "../src/toast";
(window as any).skipModelDownloadFixtureMain = true;
await import("./model-download-browser-fixture");
const f = (window as any).modelDownloadFixture;
const recipe = parseDownloadDefinition(f.definition);
recipe.manifest.target = new URLSearchParams(location.search).get("target") === "windows" ? "windows_x86_64" : "macos_aarch64";
f.definition = JSON.stringify(recipe);
const item = f.entries[0];
Object.assign(item, {
  modelName: recipe.manifest.modelName, sourceType: "download",
  download: { operationId: "33333333-3333-4333-8333-333333333333", recipeHash: "f".repeat(64) },
  reviewHash: await packageReviewHash(recipe.manifest),
  modelFiles: recipe.manifest.files.filter(file => file.role === "asr").map(file => ({ role: file.role, size: file.expectedSize, sha256: file.expectedHash })),
});
item.compatibility = { operationId: "22222222-2222-4222-8222-222222222222", packageId: item.packageId, reviewHash: item.reviewHash, adapterRevision: 1, state: "failed", errorMessage: "previous preparation failure" };
createRoot(document.getElementById("root")!).render(<main><ToastHost /><ModelInspectionPanel visible recordingActive={false} /></main>);
