-- Session provenance for profile-provisioned environments (P125) and
-- environment power states / idle policy (P126).
--
-- Design notes:
-- - origin_session_id records which session a profile provisioned the
--   environment for. It is provenance and an optional close trigger, not
--   ownership: the environment stays a universe resource that any session may
--   select and the universe may close.
-- - origin_close_with_session asks the lifecycle reconciler to close the
--   environment once the origin session's lifecycle projection is closed.
-- - No foreign key to sessions: the environment must survive session deletion
--   until its own close completes, and the sweep tolerates a missing session
--   row by treating it as closed.

ALTER TABLE environments
    ADD COLUMN IF NOT EXISTS origin_session_id text,
    ADD COLUMN IF NOT EXISTS origin_profile_id text,
    ADD COLUMN IF NOT EXISTS origin_close_with_session boolean NOT NULL DEFAULT false;

ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_origin_session_shape;
ALTER TABLE environments
    ADD CONSTRAINT environments_origin_session_shape CHECK (
        (origin_session_id IS NULL AND origin_profile_id IS NULL AND origin_close_with_session = false)
        OR (origin_session_id IS NOT NULL AND origin_session_id <> '')
    );

CREATE INDEX IF NOT EXISTS environments_origin_session_idx
    ON environments (universe_id, origin_session_id)
    WHERE origin_session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS environments_close_with_session_idx
    ON environments (universe_id)
    WHERE origin_close_with_session = true AND status NOT IN ('closing', 'closed');

-- Binding deletion is guarded by the store ("no non-closed environment may
-- reference the binding", P118); closed environments are history and must not
-- pin the binding forever. The original RESTRICT foreign key contradicted that
-- rule, so it is dropped: closed rows keep their binding_id as a historical
-- fact without referential enforcement.
ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_universe_id_binding_id_fkey;

-- Environment power states and idle policy (P126).
--
-- Design notes:
-- - desired_power is Lightspeed-owned intent; the lifecycle reconciler
--   converges the provider target toward it. The observed steady state stays
--   in `status`, which gains 'paused' and 'suspended'.
-- - idle_policy_json is the optional staged idle policy the power reaper
--   applies from the daemon's idle report. Activity itself is never stored.
-- - environment_incarnations.power_states_json records the power states the
--   provider reported for the target, observed with the target id.

ALTER TABLE environments
    ADD COLUMN IF NOT EXISTS desired_power text NOT NULL DEFAULT 'running',
    ADD COLUMN IF NOT EXISTS idle_policy_json jsonb;

ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_status_known;
ALTER TABLE environments
    ADD CONSTRAINT environments_status_known CHECK (status IN (
        'provisioning', 'booting', 'ready', 'paused', 'suspended', 'offline',
        'closing', 'closed', 'failed', 'unknown'
    ));

ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_desired_power_known;
ALTER TABLE environments
    ADD CONSTRAINT environments_desired_power_known
        CHECK (desired_power IN ('running', 'paused', 'suspended', 'stopped'));

ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_idle_policy_object;
ALTER TABLE environments
    ADD CONSTRAINT environments_idle_policy_object
        CHECK (idle_policy_json IS NULL OR jsonb_typeof(idle_policy_json) = 'object');

ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_power_provisioned_only;
ALTER TABLE environments
    ADD CONSTRAINT environments_power_provisioned_only CHECK (
        source_kind = 'provisioned'
        OR (desired_power = 'running' AND idle_policy_json IS NULL)
    );

ALTER TABLE environment_incarnations
    ADD COLUMN IF NOT EXISTS power_states_json jsonb NOT NULL DEFAULT '[]';

ALTER TABLE environment_incarnations
    DROP CONSTRAINT IF EXISTS environment_incarnations_power_states_array;
ALTER TABLE environment_incarnations
    ADD CONSTRAINT environment_incarnations_power_states_array
        CHECK (jsonb_typeof(power_states_json) = 'array');

CREATE INDEX IF NOT EXISTS environments_idle_policy_idx
    ON environments (universe_id)
    WHERE idle_policy_json IS NOT NULL AND status = 'ready';

COMMENT ON COLUMN environments.desired_power IS
    'Lightspeed-owned power intent (running|paused|suspended|stopped); converged by the lifecycle reconciler, observed state is status.';
COMMENT ON COLUMN environments.idle_policy_json IS
    'Optional staged idle policy {pauseAfterMs,suspendAfterMs,stopAfterMs,closeAfterMs}; applied by the power reaper from the daemon idle report.';
COMMENT ON COLUMN environment_incarnations.power_states_json IS
    'Provider-reported power states this target supports, observed with the target id.';
