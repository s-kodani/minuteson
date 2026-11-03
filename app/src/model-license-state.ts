export type ModelLicenseSummary = { licenseNames: string[]; summary: string; restrictions: string[]; disclosureHash: string };
export type ModelLicenseTarget = { recipeHash: string } | { packageId: string; expectedReviewHash: string };
export type ModelLicenseReview = ModelLicenseSummary & {
  targetBinding: { recipeHash: string } | { packageId: string; reviewHash: string };
  modelLabel: string; notices: { name: string; text: string; sha256: string; sourceUrl: string }[]; official: boolean;
};
const utf8 = new TextEncoder(), sha = /^[0-9a-f]{64}$/u, id = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
function fail(): never { throw Error("モデルの利用条件と選択内容が一致しません。選択し直して利用条件をご確認ください。"); }
function object(value: unknown, fields: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !fields.includes(key)) || fields.some(key => !Object.hasOwn(value, key))) fail();
  return value as Record<string, unknown>;
}
function text(value: unknown, limit: number, multiline = false): string {
  if (typeof value !== "string" || !value.trim() || utf8.encode(value).length > limit || (multiline ? /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u : /[\u0000-\u001f\u007f]/u).test(value)) fail();
  return value;
}
function hash(value: unknown): string { if (typeof value !== "string" || !sha.test(value)) fail(); return value; }
function list(value: unknown, limit: number, required = false): string[] {
  if (!Array.isArray(value) || value.length > 16 || required && !value.length) fail();
  return value.map(item => text(item, limit));
}
export function assertModelLicenseSummary(value: unknown): ModelLicenseSummary {
  const v = object(value, ["licenseNames", "summary", "restrictions", "disclosureHash"]);
  return { licenseNames: list(v.licenseNames, 256, true), summary: text(v.summary, 4096), restrictions: list(v.restrictions, 4096), disclosureHash: hash(v.disclosureHash) };
}
export function sameModelLicenseTarget(a: ModelLicenseTarget, b: ModelLicenseTarget): boolean {
  return "recipeHash" in a ? "recipeHash" in b && a.recipeHash === b.recipeHash
    : "packageId" in b && a.packageId === b.packageId && a.expectedReviewHash === b.expectedReviewHash;
}
async function digest(text: string): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", utf8.encode(text))), byte => byte.toString(16).padStart(2, "0")).join("");
}
export async function modelLicenseDisclosureHash(review: Omit<ModelLicenseReview, "disclosureHash" | "targetBinding">): Promise<string> {
  return digest(JSON.stringify(["model-license-disclosure-v1", review.modelLabel, review.licenseNames, review.summary, review.restrictions,
    review.notices.map(n => [n.name, n.text, n.sha256, n.sourceUrl]), review.official]));
}
export async function assertModelLicenseReview(value: unknown, target: ModelLicenseTarget, expected?: ModelLicenseSummary): Promise<ModelLicenseReview> {
  const v = object(value, ["targetBinding", "disclosureHash", "modelLabel", "licenseNames", "summary", "restrictions", "notices", "official"]);
  const summary = assertModelLicenseSummary({ licenseNames: v.licenseNames, summary: v.summary, restrictions: v.restrictions, disclosureHash: v.disclosureHash });
  let targetBinding: ModelLicenseReview["targetBinding"];
  if ("recipeHash" in target) {
    const binding = object(v.targetBinding, ["recipeHash"]);
    if (hash(binding.recipeHash) !== hash(target.recipeHash) || v.official !== true) fail();
    targetBinding = { recipeHash: target.recipeHash };
  } else {
    const binding = object(v.targetBinding, ["packageId", "reviewHash"]);
    if (!id.test(target.packageId) || binding.packageId !== target.packageId || hash(binding.reviewHash) !== hash(target.expectedReviewHash)) fail();
    targetBinding = { packageId: target.packageId, reviewHash: target.expectedReviewHash };
  }
  if (typeof v.official !== "boolean" || !Array.isArray(v.notices) || !v.notices.length || v.notices.length > 32) fail();
  let total = 0;
  const notices = await Promise.all(v.notices.map(async value => {
    const n = object(value, ["name", "text", "sha256", "sourceUrl"]), body = text(n.text, 400_000, true);
    total += utf8.encode(body).length;
    if (total > 2 * 1024 ** 2 || await digest(body) !== hash(n.sha256) || typeof n.sourceUrl !== "string") fail();
    if (n.sourceUrl !== "" || v.official) {
      const source = text(n.sourceUrl, 4096);
      let url: URL; try { url = new URL(source); } catch { fail(); }
      if (url.protocol !== "https:" || url.username || url.password) fail();
    }
    return { name: text(n.name, 256), text: body, sha256: n.sha256 as string, sourceUrl: n.sourceUrl as string };
  }));
  const result: ModelLicenseReview = { ...summary, targetBinding, modelLabel: text(v.modelLabel, 1024), notices, official: v.official };
  if (Array.from(result.modelLabel).length > 256 || await modelLicenseDisclosureHash(result) !== result.disclosureHash) fail();
  if (expected && (result.disclosureHash !== expected.disclosureHash || JSON.stringify(result.licenseNames) !== JSON.stringify(expected.licenseNames)
    || result.summary !== expected.summary || JSON.stringify(result.restrictions) !== JSON.stringify(expected.restrictions))) fail();
  return result;
}
