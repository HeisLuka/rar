type pendingState =
  | Pending
  | Accepted
  | Blocked(string)
  | ReresolutionRequired(string)

type outcome =
  | AcceptedOutcome(string)
  | RejectedOutcome(string)
  | ConflictOutcome(string)

type transition = {
  state: string,
  blockedReason: string,
  canonicalRevisionId: string,
  removeHead: bool,
}

let parseState = (state, blockedReason) =>
  switch state {
  | "pending" => Pending
  | "accepted" => Accepted
  | "blocked" => Blocked(blockedReason)
  | "reresolution_required" => ReresolutionRequired(blockedReason)
  | _ => raise(Failure("unsupported pending state"))
  }

let parseOutcome = (status, revisionId, code) =>
  switch status {
  | "accepted" => AcceptedOutcome(revisionId)
  | "rejected" => RejectedOutcome(code)
  | "conflict" => ConflictOutcome(code)
  | _ => raise(Failure("unsupported outcome status"))
  }

let stateName = state =>
  switch state {
  | Pending => "pending"
  | Accepted => "accepted"
  | Blocked(_) => "blocked"
  | ReresolutionRequired(_) => "reresolution_required"
  }

let stateReason = state =>
  switch state {
  | Blocked(reason)
  | ReresolutionRequired(reason) => reason
  | Pending
  | Accepted => ""
  }

let makeTransition = (state, canonicalRevisionId, removeHead) => {
  state: stateName(state),
  blockedReason: stateReason(state),
  canonicalRevisionId,
  removeHead,
}

let recordOutcome = (
  entryState,
  blockedReason,
  canonicalRevisionId,
  outcomeStatus,
  revisionId,
  code,
) => {
  let state = parseState(entryState, blockedReason)
  let outcome = parseOutcome(outcomeStatus, revisionId, code)

  switch (state, outcome) {
  | (Pending, AcceptedOutcome(nextRevisionId)) =>
    makeTransition(Accepted, nextRevisionId, true)
  | (Pending, RejectedOutcome(reason))
  | (Pending, ConflictOutcome(reason)) =>
    makeTransition(Blocked(reason), canonicalRevisionId, false)
  | _ =>
    makeTransition(state, canonicalRevisionId, false)
  }
}

let observeCanonicalAdvance = (
  entryState,
  blockedReason,
  canonicalRevisionId,
  nextRevisionId,
) => {
  let state = parseState(entryState, blockedReason)
  if nextRevisionId == canonicalRevisionId {
    makeTransition(state, canonicalRevisionId, false)
  } else {
    switch state {
    | Pending =>
      makeTransition(
        ReresolutionRequired("canonical_advanced_before_predecessor"),
        nextRevisionId,
        false,
      )
    | Accepted
    | Blocked(_)
    | ReresolutionRequired(_) =>
      makeTransition(state, nextRevisionId, false)
    }
  }
}

let isDispatchable = (entryState, blockedReason) => {
  let state = parseState(entryState, blockedReason)
  switch state {
  | Blocked(_)
  | ReresolutionRequired(_) => false
  | Pending
  | Accepted => true
  }
}
