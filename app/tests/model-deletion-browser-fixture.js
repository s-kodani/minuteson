// Synthetic panel-only IPC fixture. Not imported by the shipped application.
import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
import { ModelInspectionPanel } from "../src/model-inspection.tsx";
import { NotificationList } from "../src/toast.tsx";
import "../src/style.css";
const packageId = "11111111-1111-4111-8111-111111111111";
const entry = { packageId, modelName: "合成モデル", modelVersion: "1", engine: "whisper.cpp", engineVersion: "test only", totalBytes: 20, placementRecorded: true, ready: false, reviewHash: "a".repeat(64), compatibility: null };
const row = (relativePath, patch = {}) => ({ relativePath, directory: false, registered: true, present: true, bytes: 10, sha256: "b".repeat(64), changed: false, ...patch });
const directory = (relativePath, patch = {}) => row(relativePath, { directory: true, bytes: null, sha256: null, ...patch });
const data = window.modelDeletionFixture = { calls: [], mode: "success", pending: null, complete: () => {}, partial: false, busyCatalog: false, failCatalog: false, locked: false, recordingActive: false, mounted: true };
const root = createRoot(document.getElementById("root"));
const render = () => root.render(React.createElement(React.Fragment, null, React.createElement(NotificationList,{visible:false}), data.mounted ? React.createElement(ModelInspectionPanel, { locked: data.locked, recordingActive: data.recordingActive }) : React.createElement("p", null, "模擬アンマウント")));
data.update = patch => { Object.assign(data, patch); render(); };
window.isTauri = true;
mockWindows("main");
mockIPC(async (command, args) => {
  data.calls.push({ command, args: structuredClone(args) });
  if (command === "get_model_download_catalog") return { supported: false, entries: [], unknownEntries: 0 };
  if (command === "get_model_placements") {
    if (data.failCatalog) throw Error("模擬配置読込失敗");
    const value = { ...entry, placementRecorded: !data.partial };
    if (data.busyCatalog) return new Promise(resolve => { data.complete = () => { data.busyCatalog = false; resolve([value]); }; });
    return [value];
  }
  if (command === "get_model_profiles") return ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false }));
  if (command === "cancel_model_inspection") return null;
  if (command === "preview_model_deletion") {
    if (data.mode === "failure") throw Error("模擬リンク検出。対象を保持します");
    let entries = [directory(""), row("cli"), row("missing", { present: false, bytes: null, sha256: null }), row("unknown", { registered: false }), directory("weights"), row("weights/model.bin", { changed: true })];
    if (data.mode === "absent") entries = [directory("", { present: false }), row("cli", { present: false, bytes: null, sha256: null })];
    if (data.mode === "many") entries = [directory(""), ...Array.from({ length: 120 }, (_, n) => row(`file-${String(n).padStart(3, "0")}${n === 119 ? "日本語の長いファイル名".repeat(6) : ""}`, { registered: false }))];
    const result = { schemaVersion: 1, inspectionId: args.token, packageId: entry.packageId, reviewHash: entry.reviewHash, censusHash: "c".repeat(64), modelName: entry.modelName, modelVersion: entry.modelVersion, modelsRoot: "/synthetic/app-data/models", entries, bytes: entries.reduce((n, e) => n + (e.bytes ?? 0), 0), missingFiles: entries.filter(e => !e.directory && !e.present).length, changedFiles: entries.filter(e => e.changed).length, unregisteredFiles: entries.filter(e => !e.directory && !e.registered).length, profileReferences: 2, jobReferences: 7, activeJobs: 1, executeAllowed: false };
    if (data.mode === "forged") result.executeAllowed = true;
    if (data.mode === "wrongPackage") result.packageId = "22222222-2222-4222-8222-222222222222";
    if (data.mode === "wrongToken") result.inspectionId = "22222222-2222-4222-8222-222222222222";
    if (data.mode === "wrongTotal") result.bytes++;
    if (data.mode === "delay") return new Promise(resolve => { data.pending = args.token; data.complete = () => { data.pending = null; resolve(result); }; });
    return result;
  }
  throw Error(`Unexpected model deletion fixture command: ${command}`);
}, { shouldMockEvents: true });
render();
