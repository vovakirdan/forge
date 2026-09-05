-- Expired publisher leases are retried independently of ready pending records.
-- Keeping this partial index separate lets a bounded drain avoid a table scan
-- when a publisher process dies while holding an outbox lease.
CREATE INDEX outbox_leased_reclaim_idx
    ON outbox (lock_expires_at ASC, created_at ASC, id ASC)
    WHERE delivery_state = 'leased';
