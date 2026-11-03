---
license: apache-2.0
language:
- ja
tags:
- automatic-speech-recognition
---

# reazonspeech-k2-v2

`reazonspeech-k2-v2` is an automatic speech recognition (ASR) model
trained on [ReazonSpeech v2.0 corpus](https://huggingface.co/datasets/reazon-research/reazonspeech).

This model provides end-to-end Japanese speech recognition based on
[Next-gen Kaldi](https://k2-fsa.org/).

## Model Architecture

* Character-based RNN-T model. The total parameter count is 159.34M.

* This model utilizes an enhanced Transformer architecture called
  [Zipformer](https://arxiv.org/abs/2310.11230).

* The training recipe is available on
  [k2-fsa/icefall](https://github.com/k2-fsa/icefall/tree/master/egs/reazonspeech/ASR).

Note that this model can process Japanese audio clips up to ~30 seconds.

## Usage

We recommend to use this model through our
[reazonspeech](https://github.com/reazon-research/reazonspeech)
library.

```
from reazonspeech.k2.asr import load_model, transcribe, audio_from_path

audio = audio_from_path("speech.wav")
model = load_model()
ret = transcribe(model, audio)
print(ret.text)
```

## License

[Apaceh Licence 2.0](https://choosealicense.com/licenses/apache-2.0/)
