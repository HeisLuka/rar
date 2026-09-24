"""Document-scoped authorization and live-revocation kernel for Chaptera Cloud V1.

This is a service-layer authority contract. It deliberately does not carry
document payloads, does not mutate semantic RevisionId/StateId, and does not
choose a storage vendor. The important concurrency rule is that authorization
changes and authorized durable actions share one per-document barrier.

If an action acquires the barrier before a revoke, it may linearize before that
revoke. Once revoke() returns, no action admitted under the previous access
generation can still be executing behind the barrier.
"""

from __future__ import annotations

import base64
import copy
import hashlib
import hmac
import json
import re
import threading
from dataclasses import dataclass
from typing import Any, Callable, Dict, FrozenSet, Optional, Tuple


IDENT_RE = re.compile(r"^[A-Za-z0-9_.:@/-]{1,192}$")
SHARE_PREFIX = "chaptera-share-v1"

CAP_VIEW = "document.view"
CAP_COMMENT_READ = "comment.read"
CAP_COMMENT_WRITE = "comment.write"
CAP_EDIT = "document.edit"
CAP_EDIT_TEXT = "document.edit_text"
CAP_EDIT_GEOMETRY = "document.edit_geometry"
CAP_ASSET_UPLOAD = "asset.upload"
CAP_EXPORT = "document.export"
CAP_SHARE_MANAGE = "share.manage"
CAP_MEMBER_MANAGE = "member.manage"
CAP_DELETE = "document.delete"

ROLE_CAPABILITIES: Dict[str, FrozenSet[str]] = {
    "viewer": frozenset({CAP_VIEW}),
    "commenter": frozenset({CAP_VIEW, CAP_COMMENT_READ, CAP_COMMENT_WRITE}),
    "editor": frozenset(
        {
            CAP_VIEW,
            CAP_COMMENT_READ,
            CAP_COMMENT_WRITE,
            CAP_EDIT,
            CAP_EDIT_TEXT,
            CAP_EDIT_GEOMETRY,
            CAP_ASSET_UPLOAD,
            CAP_EXPORT,
        }
    ),
    "owner": frozenset(
        {
            CAP_VIEW,
            CAP_COMMENT_READ,
            CAP_COMMENT_WRITE,
            CAP_EDIT,
            CAP_EDIT_TEXT,
            CAP_EDIT_GEOMETRY,
            CAP_ASSET_UPLOAD,
            CAP_EXPORT,
            CAP_SHARE_MANAGE,
            CAP_MEMBER_MANAGE,
            CAP_DELETE,
        }
    ),
}

FORBIDDEN_AUDIT_KEYS = frozenset(
    {
        "text",
        "document_text",
        "story_text",
        "raw_bytes",
        "raw_pub_bytes",
        "payload",
        "content",
        "source_path",
        "filesystem_path",
        "signed_url",
        "token",
        "authorization",
        "command",
        "asset_bytes",
    }
)


class AuthzError(ValueError):
    def __init__(self, code: str, message: Optional[str] = None) -> None:
        super().__init__(message or code)
        self.code = code


class AuthzDenied(AuthzError):
    pass


def _require_ident(value: str, label: str) -> str:
    if not isinstance(value, str) or not IDENT_RE.fullmatch(value):
        raise AuthzError("invalid_identity", f"invalid {label}")
    return value


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _b64u(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def _b64u_decode(value: str) -> bytes:
    try:
        return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))
    except Exception as exc:
        raise AuthzError("invalid_share_token", "invalid share-token encoding") from exc


def _sign_share(payload: dict, secret: bytes) -> str:
    encoded = _b64u(_canonical_json(payload))
    signature = _b64u(
        hmac.new(secret, f"{SHARE_PREFIX}.{encoded}".encode("ascii"), hashlib.sha256).digest()
    )
    return f"{SHARE_PREFIX}.{encoded}.{signature}"


def _verify_share(token: str, secret: bytes) -> dict:
    try:
        prefix, encoded, supplied_signature = token.split(".", 2)
    except ValueError as exc:
        raise AuthzError("invalid_share_token", "malformed share token") from exc
    if prefix != SHARE_PREFIX:
        raise AuthzError("invalid_share_token", "share-token version mismatch")
    expected = _b64u(
        hmac.new(secret, f"{prefix}.{encoded}".encode("ascii"), hashlib.sha256).digest()
    )
    if not hmac.compare_digest(expected, supplied_signature):
        raise AuthzError("invalid_share_token", "share-token signature mismatch")
    try:
        payload = json.loads(_b64u_decode(encoded).decode("utf-8"))
    except Exception as exc:
        raise AuthzError("invalid_share_token", "invalid share-token payload") from exc
    if not isinstance(payload, dict):
        raise AuthzError("invalid_share_token", "invalid share-token payload")
    return payload


@dataclass(frozen=True)
class PrincipalGrant:
    tenant_id: str
    document_id: str
    principal_id: str
    role: str
    expires_at: Optional[int]


@dataclass(frozen=True)
class ShareGrant:
    tenant_id: str
    document_id: str
    grant_id: str
    role: str
    expires_at: int
    active: bool


@dataclass(frozen=True)
class AuthzDecision:
    tenant_id: str
    document_id: str
    principal_id: str
    capability: str
    role: str
    authz_version: int
    capabilities: FrozenSet[str]


@dataclass
class Subscription:
    subscription_id: str
    tenant_id: str
    document_id: str
    principal_id: str
    authz_version: int
    capabilities: FrozenSet[str]
    active: bool = True
    closed_reason: Optional[str] = None


class AuthzKernel:
    """In-memory reference kernel for document grants and live revocation.

    The kernel's durable semantic contract is the access-generation/barrier
    behavior. A production adapter may persist grants elsewhere, but it must
    preserve the same ordering rule.
    """

    def __init__(self) -> None:
        self._grants: Dict[Tuple[str, str, str], PrincipalGrant] = {}
        self._share_grants: Dict[Tuple[str, str, str], ShareGrant] = {}
        self._versions: Dict[Tuple[str, str], int] = {}
        self._locks: Dict[Tuple[str, str], threading.RLock] = {}
        self._locks_guard = threading.Lock()
        self._subscriptions: Dict[str, Subscription] = {}
        self._audit_events: list[dict] = []

    def _document_key(self, tenant_id: str, document_id: str) -> Tuple[str, str]:
        return (
            _require_ident(tenant_id, "tenant_id"),
            _require_ident(document_id, "document_id"),
        )

    def _document_lock(self, tenant_id: str, document_id: str) -> threading.RLock:
        key = self._document_key(tenant_id, document_id)
        with self._locks_guard:
            lock = self._locks.get(key)
            if lock is None:
                lock = threading.RLock()
                self._locks[key] = lock
            return lock

    def authz_version(self, *, tenant_id: str, document_id: str) -> int:
        key = self._document_key(tenant_id, document_id)
        with self._document_lock(*key):
            return self._versions.get(key, 0)

    def set_role(
        self,
        *,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        role: str,
        expires_at: Optional[int] = None,
    ) -> dict:
        key = self._document_key(tenant_id, document_id)
        principal_id = _require_ident(principal_id, "principal_id")
        if role not in ROLE_CAPABILITIES:
            raise AuthzError("invalid_role")
        if expires_at is not None and (
            not isinstance(expires_at, int) or isinstance(expires_at, bool) or expires_at < 0
        ):
            raise AuthzError("invalid_expiry")

        lock = self._document_lock(*key)
        with lock:
            self._grants[(key[0], key[1], principal_id)] = PrincipalGrant(
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                role=role,
                expires_at=expires_at,
            )
            version = self._bump_version_locked(key)
            self._refresh_subscriptions_locked(key, now_epoch=None)
            self._audit_locked(
                action="grant.set",
                result="allowed",
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=CAP_MEMBER_MANAGE,
                authz_version=version,
                error_code=None,
            )
            return self._barrier_receipt(key, version, "grant_set")

    def revoke(
        self,
        *,
        tenant_id: str,
        document_id: str,
        principal_id: str,
    ) -> dict:
        key = self._document_key(tenant_id, document_id)
        principal_id = _require_ident(principal_id, "principal_id")
        lock = self._document_lock(*key)
        with lock:
            self._grants.pop((key[0], key[1], principal_id), None)
            version = self._bump_version_locked(key)
            self._refresh_subscriptions_locked(key, now_epoch=None)
            self._audit_locked(
                action="grant.revoke",
                result="allowed",
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=CAP_MEMBER_MANAGE,
                authz_version=version,
                error_code=None,
            )
            return self._barrier_receipt(key, version, "grant_revoked")

    def authorize(
        self,
        *,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        capability: str,
        now_epoch: Optional[int] = None,
    ) -> AuthzDecision:
        key = self._document_key(tenant_id, document_id)
        lock = self._document_lock(*key)
        with lock:
            return self._authorize_locked(
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=capability,
                now_epoch=now_epoch,
                audit=True,
            )

    def run_authorized(
        self,
        *,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        capability: str,
        action: str,
        callback: Callable[[AuthzDecision], Any],
        now_epoch: Optional[int] = None,
    ) -> Any:
        """Authorize and execute one durable action under the same document barrier."""

        key = self._document_key(tenant_id, document_id)
        lock = self._document_lock(*key)
        with lock:
            decision = self._authorize_locked(
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=capability,
                now_epoch=now_epoch,
                audit=False,
            )
            self._audit_locked(
                action=action,
                result="allowed",
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=capability,
                authz_version=decision.authz_version,
                error_code=None,
            )
            return callback(decision)

    def subscribe(
        self,
        *,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        subscription_id: str,
        now_epoch: Optional[int] = None,
    ) -> dict:
        key = self._document_key(tenant_id, document_id)
        subscription_id = _require_ident(subscription_id, "subscription_id")
        lock = self._document_lock(*key)
        with lock:
            if subscription_id in self._subscriptions:
                raise AuthzError("subscription_exists")
            decision = self._authorize_locked(
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=CAP_VIEW,
                now_epoch=now_epoch,
                audit=False,
            )
            subscription = Subscription(
                subscription_id=subscription_id,
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                authz_version=decision.authz_version,
                capabilities=decision.capabilities,
            )
            self._subscriptions[subscription_id] = subscription
            self._audit_locked(
                action="subscription.open",
                result="allowed",
                tenant_id=key[0],
                document_id=key[1],
                principal_id=principal_id,
                capability=CAP_VIEW,
                authz_version=decision.authz_version,
                error_code=None,
            )
            return self.subscription_state(subscription_id)

    def fanout_recipients(
        self,
        *,
        tenant_id: str,
        document_id: str,
        now_epoch: Optional[int] = None,
    ) -> Tuple[str, ...]:
        key = self._document_key(tenant_id, document_id)
        lock = self._document_lock(*key)
        with lock:
            self._refresh_subscriptions_locked(key, now_epoch=now_epoch)
            return tuple(
                sorted(
                    sub.subscription_id
                    for sub in self._subscriptions.values()
                    if sub.active
                    and sub.tenant_id == key[0]
                    and sub.document_id == key[1]
                )
            )

    def subscription_state(self, subscription_id: str) -> dict:
        sub = self._subscriptions[subscription_id]
        return {
            "subscription_id": sub.subscription_id,
            "tenant_id": sub.tenant_id,
            "document_id": sub.document_id,
            "principal_id": sub.principal_id,
            "authz_version": sub.authz_version,
            "capabilities": sorted(sub.capabilities),
            "active": sub.active,
            "closed_reason": sub.closed_reason,
        }

    def issue_share_grant(
        self,
        *,
        secret: bytes,
        tenant_id: str,
        document_id: str,
        grant_id: str,
        role: str,
        now_epoch: int,
        ttl_seconds: int,
    ) -> str:
        key = self._document_key(tenant_id, document_id)
        grant_id = _require_ident(grant_id, "grant_id")
        if role not in {"viewer", "commenter"}:
            raise AuthzError("share_role_not_allowed")
        if not isinstance(now_epoch, int) or isinstance(now_epoch, bool) or now_epoch < 0:
            raise AuthzError("invalid_issue_time")
        if (
            not isinstance(ttl_seconds, int)
            or isinstance(ttl_seconds, bool)
            or ttl_seconds <= 0
            or ttl_seconds > 86_400
        ):
            raise AuthzError("invalid_share_ttl")
        lock = self._document_lock(*key)
        with lock:
            expires_at = now_epoch + ttl_seconds
            share = ShareGrant(
                tenant_id=key[0],
                document_id=key[1],
                grant_id=grant_id,
                role=role,
                expires_at=expires_at,
                active=True,
            )
            self._share_grants[(key[0], key[1], grant_id)] = share
            version = self._bump_version_locked(key)
            payload = {
                "v": 1,
                "tenant_id": key[0],
                "document_id": key[1],
                "grant_id": grant_id,
                "role": role,
                "exp": expires_at,
            }
            self._audit_locked(
                action="share.issue",
                result="allowed",
                tenant_id=key[0],
                document_id=key[1],
                principal_id=f"share:{grant_id}",
                capability=CAP_SHARE_MANAGE,
                authz_version=version,
                error_code=None,
            )
            return _sign_share(payload, secret)

    def revoke_share_grant(
        self,
        *,
        tenant_id: str,
        document_id: str,
        grant_id: str,
    ) -> dict:
        key = self._document_key(tenant_id, document_id)
        grant_id = _require_ident(grant_id, "grant_id")
        lock = self._document_lock(*key)
        with lock:
            share_key = (key[0], key[1], grant_id)
            prior = self._share_grants.get(share_key)
            if prior is not None:
                self._share_grants[share_key] = ShareGrant(
                    tenant_id=prior.tenant_id,
                    document_id=prior.document_id,
                    grant_id=prior.grant_id,
                    role=prior.role,
                    expires_at=prior.expires_at,
                    active=False,
                )
            version = self._bump_version_locked(key)
            return self._barrier_receipt(key, version, "share_revoked")

    def authorize_share_token(
        self,
        *,
        token: str,
        secret: bytes,
        tenant_id: str,
        document_id: str,
        capability: str,
        now_epoch: int,
    ) -> AuthzDecision:
        key = self._document_key(tenant_id, document_id)
        lock = self._document_lock(*key)
        with lock:
            payload = _verify_share(token, secret)
            if payload.get("v") != 1:
                raise AuthzDenied("invalid_share_token")
            if payload.get("tenant_id") != key[0] or payload.get("document_id") != key[1]:
                raise AuthzDenied("share_scope_mismatch")
            grant_id = payload.get("grant_id")
            if not isinstance(grant_id, str):
                raise AuthzDenied("invalid_share_token")
            share = self._share_grants.get((key[0], key[1], grant_id))
            if share is None or not share.active:
                raise AuthzDenied("share_revoked")
            if payload.get("role") != share.role or payload.get("exp") != share.expires_at:
                raise AuthzDenied("share_state_mismatch")
            if not isinstance(now_epoch, int) or isinstance(now_epoch, bool) or now_epoch < 0:
                raise AuthzError("invalid_now")
            if now_epoch >= share.expires_at:
                raise AuthzDenied("share_expired")
            capabilities = ROLE_CAPABILITIES[share.role]
            if capability not in capabilities:
                raise AuthzDenied("capability_denied")
            return AuthzDecision(
                tenant_id=key[0],
                document_id=key[1],
                principal_id=f"share:{grant_id}",
                capability=capability,
                role=share.role,
                authz_version=self._versions.get(key, 0),
                capabilities=capabilities,
            )

    def audit_events(self) -> list[dict]:
        return copy.deepcopy(self._audit_events)

    def _authorize_locked(
        self,
        *,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        capability: str,
        now_epoch: Optional[int],
        audit: bool,
    ) -> AuthzDecision:
        principal_id = _require_ident(principal_id, "principal_id")
        if capability not in set().union(*ROLE_CAPABILITIES.values()):
            raise AuthzError("unknown_capability")
        key = (tenant_id, document_id)
        version = self._versions.get(key, 0)
        grant = self._grants.get((tenant_id, document_id, principal_id))
        error_code: Optional[str] = None

        if grant is None:
            error_code = "grant_missing"
        elif grant.expires_at is not None:
            if now_epoch is None:
                error_code = "time_required"
            elif not isinstance(now_epoch, int) or isinstance(now_epoch, bool) or now_epoch < 0:
                raise AuthzError("invalid_now")
            elif now_epoch >= grant.expires_at:
                error_code = "grant_expired"

        capabilities: FrozenSet[str] = frozenset()
        role = "none"
        if error_code is None and grant is not None:
            role = grant.role
            capabilities = ROLE_CAPABILITIES[grant.role]
            if capability not in capabilities:
                error_code = "capability_denied"

        if error_code is not None:
            if audit:
                self._audit_locked(
                    action="authorize",
                    result="denied",
                    tenant_id=tenant_id,
                    document_id=document_id,
                    principal_id=principal_id,
                    capability=capability,
                    authz_version=version,
                    error_code=error_code,
                )
            raise AuthzDenied(error_code)

        decision = AuthzDecision(
            tenant_id=tenant_id,
            document_id=document_id,
            principal_id=principal_id,
            capability=capability,
            role=role,
            authz_version=version,
            capabilities=capabilities,
        )
        if audit:
            self._audit_locked(
                action="authorize",
                result="allowed",
                tenant_id=tenant_id,
                document_id=document_id,
                principal_id=principal_id,
                capability=capability,
                authz_version=version,
                error_code=None,
            )
        return decision

    def _refresh_subscriptions_locked(
        self,
        key: Tuple[str, str],
        *,
        now_epoch: Optional[int],
    ) -> None:
        version = self._versions.get(key, 0)
        for sub in self._subscriptions.values():
            if (
                not sub.active
                or sub.tenant_id != key[0]
                or sub.document_id != key[1]
            ):
                continue
            try:
                decision = self._authorize_locked(
                    tenant_id=key[0],
                    document_id=key[1],
                    principal_id=sub.principal_id,
                    capability=CAP_VIEW,
                    now_epoch=now_epoch,
                    audit=False,
                )
            except AuthzDenied as exc:
                sub.active = False
                sub.closed_reason = exc.code
                sub.authz_version = version
                sub.capabilities = frozenset()
            else:
                sub.authz_version = decision.authz_version
                sub.capabilities = decision.capabilities

    def _bump_version_locked(self, key: Tuple[str, str]) -> int:
        version = self._versions.get(key, 0) + 1
        self._versions[key] = version
        return version

    @staticmethod
    def _barrier_receipt(key: Tuple[str, str], version: int, outcome: str) -> dict:
        return {
            "protocol_version": "chaptera.authz-barrier-receipt.v1",
            "tenant_id": key[0],
            "document_id": key[1],
            "authz_version": version,
            "outcome": outcome,
            "active_session_barrier_complete": True,
        }

    def _audit_locked(
        self,
        *,
        action: str,
        result: str,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        capability: str,
        authz_version: int,
        error_code: Optional[str],
    ) -> None:
        for name, value in {
            "action": action,
            "result": result,
            "tenant_id": tenant_id,
            "document_id": document_id,
            "principal_id": principal_id,
            "capability": capability,
        }.items():
            _require_ident(value, name)
        event = {
            "protocol_version": "chaptera.authz-audit.v1",
            "tenant_id": tenant_id,
            "document_id": document_id,
            "principal_id": principal_id,
            "action": action,
            "result": result,
            "capability": capability,
            "authz_version": authz_version,
            "error_code": error_code,
        }
        if FORBIDDEN_AUDIT_KEYS.intersection(event):
            raise AssertionError("forbidden document payload field in authz audit")
        self._audit_events.append(event)
