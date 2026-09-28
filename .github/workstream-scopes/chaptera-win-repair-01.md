# CHAPTERA-WIN-REPAIR-01 — first slice

Defines the explicit RepairRequired -> authenticated installer -> self-check -> Repaired state contract.

This slice preserves broken-tree evidence until success and fails closed on wrong product/architecture/channel or installer/self-check failure.

It does not execute installers, alter Windows registration, or fetch network content yet. Those remain follow-up adapters over this contract and the existing trusted update/installer authority.
