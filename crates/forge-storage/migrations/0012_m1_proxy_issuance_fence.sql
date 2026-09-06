-- A missing key observed during revoke does not settle a concurrent/lost issue.
ALTER TABLE run_proxy_keys ADD COLUMN issuance_pending BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE run_proxy_keys ADD COLUMN next_revocation_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp();
CREATE INDEX run_proxy_keys_pending_revocation_idx ON run_proxy_keys(next_revocation_attempt_at)
    WHERE revoked_at IS NULL OR issuance_pending;
