-- P133: immutable grant exposure plus service-lease audit metadata. OAuth
-- flows retain the requested exposure so callback-created grants preserve the
-- creation-time choice.

ALTER TABLE auth_grants
    ADD COLUMN IF NOT EXISTS exposure text NOT NULL DEFAULT 'brokered',
    ADD COLUMN IF NOT EXISTS last_leased_at_ms bigint,
    ADD COLUMN IF NOT EXISTS lease_count bigint NOT NULL DEFAULT 0;

ALTER TABLE auth_grants
    DROP CONSTRAINT IF EXISTS auth_grants_exposure_known,
    ADD CONSTRAINT auth_grants_exposure_known
        CHECK (exposure IN ('brokered', 'retrievable')),
    DROP CONSTRAINT IF EXISTS auth_grants_last_leased_at_ms_nonnegative,
    ADD CONSTRAINT auth_grants_last_leased_at_ms_nonnegative
        CHECK (last_leased_at_ms IS NULL OR last_leased_at_ms >= 0),
    DROP CONSTRAINT IF EXISTS auth_grants_lease_count_nonnegative,
    ADD CONSTRAINT auth_grants_lease_count_nonnegative
        CHECK (lease_count >= 0);

ALTER TABLE auth_flows
    ADD COLUMN IF NOT EXISTS grant_exposure text NOT NULL DEFAULT 'brokered';

ALTER TABLE auth_flows
    DROP CONSTRAINT IF EXISTS auth_flows_grant_exposure_known,
    ADD CONSTRAINT auth_flows_grant_exposure_known
        CHECK (grant_exposure IN ('brokered', 'retrievable'));
