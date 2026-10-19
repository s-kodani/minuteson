import type {SpeakerIdentification} from "./transcript-state";
import {identificationText} from "./speaker-completion-state";

export function IdentificationBadge({value}:{value?:SpeakerIdentification|null}) {
  const text=identificationText(value);
  return text?<details className={`speaker-identification${text.needsReview?" needs-review":""}`}>
    <summary>{text.label}</summary><p>{text.detail}</p>
  </details>:null;
}
