type recoveryState =
  | Prepared
  | SentUnknown
  | Rejected(string)
  | Quarantined(string)

type lookupOutcome =
  | AcceptedLookup(string)
  | RejectedLookup(string)
  | NotFound

type decision = {
  action: string,
  reason: string,
  revisionId: string,
  dependencyCode: string,
}

let parseState = (state, reason) =>
  switch state {
  | "prepared" => Prepared
  | "sent_unknown" => SentUnknown
  | "rejected" => Rejected(reason)
  | "quarantined" => Quarantined(reason)
  | _ => throw(Failure("unsupported recovery state"))
  }

let parseLookup = (status, revisionId, code) =>
  switch status {
  | "accepted" => AcceptedLookup(revisionId)
  | "rejected" => RejectedLookup(code)
  | "not_found" => NotFound
  | _ => throw(Failure("unsupported lookup outcome"))
  }

let make = (~action, ~reason="", ~revisionId="", ~dependencyCode="") => {
  action,
  reason,
  revisionId,
  dependencyCode,
}

let planOne = (
  stateName,
  stateReason,
  principalMatches,
  authzAllowed,
  schemaMatches,
  appMatches,
  commandMatches,
  lifecycleMatches,
  currentRevisionMatches,
  lookupStatus,
  lookupRevisionId,
  lookupCode,
  dependencyPresent,
  dependencyStatus,
  dependencyCode,
) => {
  let state = parseState(stateName, stateReason)

  switch state {
  | Quarantined(reason) => make(~action="quarantined", ~reason)
  | _ if !principalMatches => make(~action="quarantined", ~reason="auth_scope_changed")
  | _ if !authzAllowed => make(~action="quarantined", ~reason="authz_denied")
  | _ if !schemaMatches => make(~action="quarantined", ~reason="schema_version_mismatch")
  | _ if !appMatches => make(~action="quarantined", ~reason="app_version_mismatch")
  | _ if !commandMatches =>
    make(~action="quarantined", ~reason="command_semantic_version_mismatch")
  | _ if !lifecycleMatches =>
    make(~action="quarantined", ~reason="lifecycle_generation_mismatch")
  | Rejected(code) => make(~action="surface_rejection", ~reason=code)
  | SentUnknown =>
    switch parseLookup(lookupStatus, lookupRevisionId, lookupCode) {
    | AcceptedLookup(revisionId) => make(~action="resolved_accepted", ~revisionId)
    | RejectedLookup(code) => make(~action="surface_rejection", ~reason=code)
    | NotFound =>
      if !currentRevisionMatches {
        make(~action="refresh_required", ~reason="stale_base_revision")
      } else if dependencyPresent {
        switch parseLookup(dependencyStatus, "", dependencyCode) {
        | NotFound => make(~action="blocked_dependency")
        | RejectedLookup(code) =>
          make(~action="blocked_dependency_rejected", ~dependencyCode=code)
        | AcceptedLookup(_) => make(~action="retry_exact_identity")
        }
      } else {
        make(~action="retry_exact_identity")
      }
    }
  | Prepared =>
    if !currentRevisionMatches {
      make(~action="refresh_required", ~reason="stale_base_revision")
    } else if dependencyPresent {
      switch parseLookup(dependencyStatus, "", dependencyCode) {
      | NotFound => make(~action="blocked_dependency")
      | RejectedLookup(code) =>
        make(~action="blocked_dependency_rejected", ~dependencyCode=code)
      | AcceptedLookup(_) => make(~action="retry_exact_identity")
      }
    } else {
      make(~action="retry_exact_identity")
    }
  }
}
