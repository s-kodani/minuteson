import { modelLicenseDisclosureHash, type ModelLicenseReview, type ModelLicenseTarget } from "../src/model-license-state.ts";
export const modelLicenseSummaryFixture = { licenseNames: ["Synthetic license"], summary: "Synthetic metadata fixture only.", restrictions: [], disclosureHash: "a".repeat(64) };

export async function modelLicenseFixture(modelLabel = "合成モデル", target: ModelLicenseTarget = { recipeHash: "a".repeat(64) }, restricted = false, policyRevision = 0): Promise<ModelLicenseReview> {
  const text = "SYNTHETIC LICENSE — browser/unit fixture only.\n<script>window.licenseExecuted = true</script>\n" + "Long offline license paragraph.\n".repeat(160);
  const sha256 = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))), byte => byte.toString(16).padStart(2, "0")).join("");
  const official = "recipeHash" in target;
  const review = { modelLabel, licenseNames: official ? ["合成モデル利用条件", "合成実行エンジン利用条件"] : ["条件未確認"], summary: official ? `画面検証専用の合成利用条件です。${policyRevision ? `改訂${policyRevision}` : ""}` : "合成提供元の申告・条件未確認です。", restrictions: restricted ? ["年商1,000万米ドル（US$10M）を基準とする商用利用の条件があります。関連会社を含む適用範囲は第1条・第5条を確認してください。"] : [],
    notices: [{ name: "モデルと実行エンジンの合成全文", text, sha256, sourceUrl: official ? "https://example.com/synthetic-license" : "" }], official };
  return { ...review, disclosureHash: await modelLicenseDisclosureHash(review), targetBinding: "recipeHash" in target ? target : { packageId: target.packageId, reviewHash: target.expectedReviewHash } };
}
