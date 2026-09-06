# Frame failure and recovery contract

This is the v1.0 contract for classified paint-backend failures. Finite queue,
upload, IR, and memory budgets are supported; accepting arbitrarily large frames
is not required. Their limits must not turn temporary pressure into device loss
or certify an incomplete image as safe to present.

## Application-visible outcomes

`Error::RenderFailure(RenderFailure { kind, reason })` preserves the recovery
classification and diagnostic cause through the paint backend and pipeline.
`reason` is human-readable, not a string-parsing API. The application runner calls
`Application::on_render_error(&WindowContext, &RenderFailure)` for these failures.

| Kind | Safety guarantee | Runner policy |
| --- | --- | --- |
| `Busy` | Previous display unchanged; all accepted work from the discarded frame retired successfully. | Keep processing input and retry a newly encoded full frame on a later event-loop tick. No tight submission retry loop. |
| `Rejected` | Same retirement/display guarantee; the input, support, or resource limit rejected the frame. | Keep processing input. Do not retry unchanged input automatically; wait for scene invalidation. The application may change state or reduce its rendering load in the hook. |
| `RecoveryRequired` | Retirement is failed or uncertain; no resource-reuse guarantee. | Notify the application, then return the error from the runner. Do not render into or publish the uncertain target. Recreate the backend/window through a new runner invocation before retrying. Device loss may require recovery outside the application. |

Unknown/unclassified backend errors remain fatal; they must not be downgraded to
recoverable rejection. The default hook logs non-transient failures. Failed
frames never call `on_frame_presented`, consume a successful presentation slot,
or acknowledge a new displayed frame. Busy retries still honor suspension and
frame grants. A continually changing scene may produce repeated rejections;
the runner does not choose a lower render distance or LOD on the application's
behalf.

## SGFX and ScarletUI responsibilities

SGFX owns native packetization, queue capacity, staging storage, ordered dispatch,
and whole-stream completion. ScarletUI owns frame boundaries, cache validity,
presentation, and external image leases. ScarletUI does not subdivide native
uploads or meshes to match a transport's packet size.

SGFX `SubmitError::Rejected` certifies **only that particular call** was not
accepted. It does not roll back earlier calls in the frame or certify the health
of the context. The native integration permits continued use only if:

1. CPU lowering failed, receipt storage could not be reserved, admission returned
   Busy, or SGFX positively classifies the rejection as recoverable.
2. `FrameExecutor::discard()` observes explicit successful completion of every
   earlier accepted stream. Pending, observation failure, and failed-prefix
   acceptance never satisfy this condition.

After successful discard, the failed target is not committed, and the previous
front image and SWS lease remain unchanged. Canvas pixel-revision caches are
invalidated because an earlier pass could have changed them. Successfully
retired mesh/texture uploads remain cached. The next frame is fully encoded and
repainted from current scene state, with full presentation damage; no raw command
stream or accepted prefix is replayed.

`FrameExecutor::wait()` cannot turn an aborted frame into a presentable one.
The separate `discard()` operation certifies retirement only and permanently
prevents further submission through that frame executor. GPU failure also
invalidates the native paint backend; it retains its session and images while
alive, and backend/kernel-owned work retention remains responsible for safe
teardown. Dropping a receipt is neither cancellation nor completion.

The legacy synchronous Adreno path has no tracked retirement contract. Its
execution failures remain recovery-required, not recoverable rejections. Other
paint backends may report classified failures only when they satisfy the same
guarantees; the older generic RenderError continues to be fatal.

## Limits

VirGL currently bounds a logical submission to 64 MiB of lowered command bytes.
An individual stream exceeding that size is permanently rejected; waiting cannot
make that same stream fit. Aggregate pending-queue pressure is Busy instead.
This contract does not increase the limit or implement arbitrary-size streaming.
It ensures the application can receive the rejection and continue processing
input/state changes without publishing partial pixels or exiting on overload.

## Regression coverage

- Accepted prefix followed by rejection: retire once, never replay, new frame works.
- Pending/failed prefix retirement and partial acceptance: never certify reuse.
- Canvas validity reset without reuploading successfully retired meshes.
- Real pipeline with an injected backend: previous presentation count unchanged
  on failure, typed notification reaches the app, Busy retries on another tick,
  Rejected waits for invalidation, and the next successful frame has full damage.
- GPU failure reaches the hook and propagates out of the runner without retry.

These are deterministic host tests plus native compile checks. They do not
constitute QEMU or Boxcraft runtime verification; runtime confirmation is user-run.
