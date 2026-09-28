# CHAPTERA-WIN-UPDATE-ACCEPT-01

Fresh-main W2 acceptance over merged updater-core and the landed first Windows acceptance slice.

## Goal
Prove the updater composes with the actual installed Chaptera Reader product boundary, not only synthetic fixture trees.

## Reused lower-level authorities
Do not duplicate proofs already owned elsewhere:
- chaptera-update-engine transaction acceptance owns journal/crash/rename seams;
- chaptera-update-engine staging acceptance owns tree-manifest/tamper/bounds;
- Chaptera Reader Windows V0 owns the source-free Reader binary and pinned real-PUB smoke;
- Chaptera Reader Windows installer V1 owns the stable current/.staging/.rollback layout plus Open With/no-default-steal and uninstall semantics.

## This slice
On windows-latest:
1. build the real reader-only desktop binary;
2. compile and run the existing stable-AppId Inno installer;
3. preserve a foreign pre-existing .pub default and external state sentinel;
4. run the pinned Apache POI SampleNewsletter PUB through the installed Reader;
5. apply authenticated payload-swap policy A -> B using the real installed current/ tree;
6. run real Reader health smoke on B and confirm it;
7. apply C, run real Reader smoke, then inject a post-smoke health rejection;
8. require rollback to restore the confirmed B tree byte-for-byte;
9. run the real PUB smoke again after rollback;
10. verify Open With, foreign .pub default, source PUB and external user state are unchanged;
11. run the ordinary Reader uninstaller and prove updater-owned generations/journals are cleaned while external state/default remain.

## Non-goals
- no production signing credentials;
- no second installer or registry authority;
- no duplicate crash/tamper matrix;
- no public update repository;
- no document-model changes.
