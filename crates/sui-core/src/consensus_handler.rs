// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::authority::authority_store_tables::ExecutionIndicesWithHash;
use crate::authority::AuthorityState;
use crate::consensus_adapter::ConsensusListenerMessage;
use async_trait::async_trait;
use narwhal_executor::{ExecutionIndices, ExecutionState};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use sui_types::messages::ConsensusTransaction;
use tokio::sync::mpsc;
use tracing::{debug, instrument, warn};

pub struct ConsensusHandler {
    state: Arc<AuthorityState>,
    sender: mpsc::Sender<ConsensusListenerMessage>,
    last_seen: Mutex<ExecutionIndicesWithHash>,
}

impl ConsensusHandler {
    pub fn new(state: Arc<AuthorityState>, sender: mpsc::Sender<ConsensusListenerMessage>) -> Self {
        let last_seen = Mutex::new(Default::default());
        Self {
            state,
            sender,
            last_seen,
        }
    }

    fn update_hash(&self, index: ExecutionIndices, v: &[u8]) -> Option<ExecutionIndicesWithHash> {
        let mut last_seen_guard = self
            .last_seen
            .try_lock()
            .expect("Should not have contention on ExecutionState::update_hash");
        if last_seen_guard.index <= index {
            return None;
        }
        let previous_hash = last_seen_guard.hash;
        let mut hasher = DefaultHasher::new();
        previous_hash.hash(&mut hasher);
        v.hash(&mut hasher);
        let hash = hasher.finish();
        // Log hash for every certificate
        if index.next_transaction_index == 1 && index.next_batch_index == 1 {
            debug!(
                "Integrity hash for consensus output at certificate {} is {:016x}",
                index.next_certificate_index, hash
            );
        }
        let last_seen = ExecutionIndicesWithHash { index, hash };
        *last_seen_guard = last_seen.clone();
        Some(last_seen)
    }
}

#[async_trait]
impl ExecutionState for ConsensusHandler {
    /// This function will be called by Narwhal, after Narwhal sequenced this certificate.
    #[instrument(level = "trace", skip_all)]
    async fn handle_consensus_transaction(
        &self,
        // TODO [2533]: use this once integrating Narwhal reconfiguration
        consensus_output: &Arc<narwhal_consensus::ConsensusOutput>,
        consensus_index: ExecutionIndices,
        serialized_transaction: Vec<u8>,
    ) {
        let consensus_index = self.update_hash(consensus_index, &serialized_transaction);
        let consensus_index = if let Some(consensus_index) = consensus_index {
            consensus_index
        } else {
            debug!(
                "Ignore consensus transaction at index {:?} as it appear to be already processed",
                consensus_index
            );
            return;
        };
        let transaction =
            match bincode::deserialize::<ConsensusTransaction>(&serialized_transaction) {
                Ok(transaction) => transaction,
                Err(err) => {
                    warn!(
                        "Ignoring malformed transaction (failed to deserialize) from {}: {}",
                        consensus_output.certificate.header.author, err
                    );
                    return;
                }
            };
        let sequenced_transaction = SequencedConsensusTransaction {
            consensus_output: consensus_output.clone(),
            consensus_index,
            transaction,
        };
        let verified_transaction = match self
            .state
            .verify_consensus_transaction(sequenced_transaction)
        {
            Ok(verified_transaction) => verified_transaction,
            Err(()) => return,
        };
        self.state
            .handle_consensus_transaction(verified_transaction)
            .await
            .expect("Unrecoverable error in consensus handler");
        if self
            .sender
            .send(ConsensusListenerMessage::Processed(serialized_transaction))
            .await
            .is_err()
        {
            warn!("Consensus handler outbound channel closed");
        }
    }

    async fn load_execution_indices(&self) -> ExecutionIndices {
        let index_with_hash = self
            .state
            .database
            .last_consensus_index()
            .expect("Failed to load consensus indices");
        *self
            .last_seen
            .try_lock()
            .expect("Should not have contention on ExecutionState::load_execution_indices") =
            index_with_hash.clone();
        index_with_hash.index
    }
}

pub struct SequencedConsensusTransaction {
    pub consensus_output: Arc<narwhal_consensus::ConsensusOutput>,
    pub consensus_index: ExecutionIndicesWithHash,
    pub transaction: ConsensusTransaction,
}

pub struct VerifiedSequencedConsensusTransaction(pub SequencedConsensusTransaction);

#[cfg(test)]
impl VerifiedSequencedConsensusTransaction {
    pub fn new_test(transaction: ConsensusTransaction) -> Self {
        Self(SequencedConsensusTransaction::new_test(transaction))
    }
}

#[cfg(test)]
impl SequencedConsensusTransaction {
    pub fn new_test(transaction: ConsensusTransaction) -> Self {
        Self {
            transaction,
            consensus_output: Default::default(),
            consensus_index: Default::default(),
        }
    }
}
