//! Fenced management intent never releases a physical execution reservation.
use uuid::Uuid;

use crate::{StorageError, StorageTransaction, u64_to_i64};

impl StorageTransaction<'_> {
    /// Revokes the exact logical Lease after an explicit forced-stop request.
    pub async fn revoke_run_lease(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE leases l SET lease_state='revoked', revoked_at=clock_timestamp(), revocation_reason='manager_force_stop', revision=l.revision+1 FROM runs r WHERE r.id=$1 AND r.lease_fencing_token=$2 AND r.environment_epoch=$3 AND r.desired_state='force_stop_requested' AND forge_lease_owns_run(l,r) AND l.lease_state='active'",
        )
        .bind(run)
        .bind(u64_to_i64(fence, "run.lease_fencing_token")?)
        .bind(u64_to_i64(epoch, "run.environment_epoch")?)
        .execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }
}
