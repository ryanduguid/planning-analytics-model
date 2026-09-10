# Repository instructions

Read [CONTRIBUTING.md](CONTRIBUTING.md) before changing the engine, model or
examples. For model changes, preserve the manifest coverage and feeder rules
documented there. Expected monetary results must show independent arithmetic.

When changing a statutory input, update its primary-source evidence and retrieval
date in [docs/model-assumptions.md](docs/model-assumptions.md). The example rates
are dated planning inputs; they do not establish current statutory treatment.
Use fabricated examples only.

Before handoff, run the relevant checks from CONTRIBUTING.md and
[ci.yml](.github/workflows/ci.yml). For native agreement claims, read
[docs/native-comparison.md](docs/native-comparison.md) and retain the required
TM1 evidence. Passing offline tests does not establish native server agreement.
