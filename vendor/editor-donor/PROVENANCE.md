# Historical desktop/editor donor slice

Source provenance: `HeisLuka/yab@main` at the EDITOR-DESKTOP-APP-01 bootstrap date.

This directory is a bounded source donor for the first current Chaptera desktop
application in `HeisLuka/rar`. It is not a second active repository and does not
make historical `yab` an execution dependency.

The slice contains only the legacy editor/interaction/export crates needed by
the current Rar-owned desktop shell. Reverse/viewer parsing remains in the
separate read-only `vendor/producer-a` slice. Current visual-instance mutation
authority remains `crates/chaptera-scene-instance` in the Rar root workspace.

Do not add native PUB Save claims or revive old NodeId-only projected mutation
behavior here.
