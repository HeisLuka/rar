"""Chaptera Cloud quota/reservation V1 public reference contract."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, Tuple


class QuotaConflict(ValueError):
    pass


class QuotaRejected(ValueError):
    pass


@dataclass
class Reservation:
    tenant_id: str
    reservation_id: str
    work_class: str
    amount: int
    lease_generation: int = 1
    released: bool = False


class CloudQuotaV1:
    """Atomic in-memory reference model for admission semantics.

    Capacity is split into:
      - semantic_headroom: only interactive semantic work may consume it;
      - shared_capacity: interactive/export/background may consume subject to class caps.

    This is a contract kernel, not a distributed implementation.
    """

    VALID_CLASSES = {"interactive", "export", "background"}

    def __init__(
        self,
        *,
        shared_capacity: int,
        semantic_headroom: int,
        export_cap: int,
        background_cap: int,
    ) -> None:
        for value in (shared_capacity, semantic_headroom, export_cap, background_cap):
            if not isinstance(value, int) or value < 0:
                raise ValueError("capacity values must be non-negative integers")
        self.shared_capacity = shared_capacity
        self.semantic_headroom = semantic_headroom
        self.class_caps = {
            "export": export_cap,
            "background": background_cap,
        }
        self._reservations: Dict[Tuple[str, str], Reservation] = {}

    def _active(self):
        return [r for r in self._reservations.values() if not r.released]

    def usage(self, tenant_id: str) -> dict:
        rows = [r for r in self._active() if r.tenant_id == tenant_id]
        interactive = sum(r.amount for r in rows if r.work_class == "interactive")
        export = sum(r.amount for r in rows if r.work_class == "export")
        background = sum(r.amount for r in rows if r.work_class == "background")
        # Interactive fills shared first, then protected semantic headroom.
        shared_interactive = min(interactive, self.shared_capacity)
        protected_interactive = max(0, interactive - shared_interactive)
        shared_total = min(self.shared_capacity, interactive) + export + background
        return {
            "interactive": interactive,
            "export": export,
            "background": background,
            "shared_total": shared_total,
            "protected_interactive": protected_interactive,
        }

    def reserve(
        self,
        *,
        tenant_id: str,
        reservation_id: str,
        work_class: str,
        amount: int,
    ) -> dict:
        if work_class not in self.VALID_CLASSES:
            raise ValueError("invalid work_class")
        if not isinstance(amount, int) or amount <= 0:
            raise ValueError("amount must be positive integer")
        key = (tenant_id, reservation_id)
        prior = self._reservations.get(key)
        if prior is not None:
            if prior.work_class != work_class or prior.amount != amount:
                raise QuotaConflict("reservation_conflict")
            if prior.released:
                raise QuotaConflict("reservation_already_released")
            return self.snapshot(prior)

        usage = self.usage(tenant_id)
        if work_class == "export":
            if usage["export"] + amount > self.class_caps["export"]:
                raise QuotaRejected("export_concurrency_quota")
            if usage["shared_total"] + amount > self.shared_capacity:
                raise QuotaRejected("shared_capacity_exhausted")
        elif work_class == "background":
            if usage["background"] + amount > self.class_caps["background"]:
                raise QuotaRejected("background_budget_paused")
            if usage["shared_total"] + amount > self.shared_capacity:
                raise QuotaRejected("background_budget_paused")
        else:
            total_interactive_after = usage["interactive"] + amount
            # Interactive may use shared capacity plus protected semantic headroom.
            if total_interactive_after > self.shared_capacity + self.semantic_headroom:
                raise QuotaRejected("semantic_headroom_exhausted")

        reservation = Reservation(
            tenant_id=tenant_id,
            reservation_id=reservation_id,
            work_class=work_class,
            amount=amount,
        )
        self._reservations[key] = reservation
        return self.snapshot(reservation)

    def release(
        self,
        *,
        tenant_id: str,
        reservation_id: str,
        expected_lease_generation: int,
    ) -> dict:
        key = (tenant_id, reservation_id)
        reservation = self._reservations[key]
        if reservation.lease_generation != expected_lease_generation:
            raise QuotaConflict("stale_lease_generation")
        if reservation.released:
            raise QuotaConflict("already_released")
        reservation.released = True
        return self.snapshot(reservation)

    def renew_lease(
        self,
        *,
        tenant_id: str,
        reservation_id: str,
        expected_lease_generation: int,
    ) -> dict:
        reservation = self._reservations[(tenant_id, reservation_id)]
        if reservation.released:
            raise QuotaConflict("reservation_released")
        if reservation.lease_generation != expected_lease_generation:
            raise QuotaConflict("stale_lease_generation")
        reservation.lease_generation += 1
        return self.snapshot(reservation)

    def expire_stale(
        self,
        *,
        tenant_id: str,
        reservation_id: str,
        observed_lease_generation: int,
    ) -> dict:
        """Reconcile a stale reservation by releasing exactly the observed lease."""
        reservation = self._reservations[(tenant_id, reservation_id)]
        if reservation.released:
            return self.snapshot(reservation)
        if reservation.lease_generation != observed_lease_generation:
            raise QuotaConflict("lease_advanced")
        reservation.released = True
        return self.snapshot(reservation)

    @staticmethod
    def snapshot(reservation: Reservation) -> dict:
        return {
            "tenant_id": reservation.tenant_id,
            "reservation_id": reservation.reservation_id,
            "work_class": reservation.work_class,
            "amount": reservation.amount,
            "lease_generation": reservation.lease_generation,
            "released": reservation.released,
        }
