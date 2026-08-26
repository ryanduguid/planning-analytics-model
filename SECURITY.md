# Security

## Supported version

The latest commit on the default branch. There is no backport branch.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting on this repository. Do not open a
public issue.

Expect an acknowledgement within seven days. If the report is valid, the fix and
a note of what was affected land on the default branch.

## What this project does and does not do

The runtime imports nothing outside the Python standard library. It makes no
network calls, reads no credentials and writes no files unless you ask it to.
It reads two things: the model source under `model/` and whatever CSV files you
point it at.

Model source is data, not code. The loader refuses a link that climbs out of the
model root, so a manifest or an object file cannot reach an arbitrary path on
the machine that loads it. Rule text is parsed, never executed.

The TurboIntegrator processes under `model/processes/` are source for a Planning
Analytics server to run. Nothing here executes them. Read them before you deploy
them, the same as any script you did not write.
