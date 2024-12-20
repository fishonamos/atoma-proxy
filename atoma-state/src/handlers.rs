use atoma_sui::events::{
    AtomaEvent, NewKeyRotationEvent, NewStackSettlementAttestationEvent,
    NodePublicKeyCommittmentEvent, NodeRegisteredEvent, NodeSubscribedToTaskEvent,
    NodeSubscriptionUpdatedEvent, NodeUnsubscribedFromTaskEvent, StackAttestationDisputeEvent,
    StackCreatedEvent, StackSettlementTicketClaimedEvent, StackSettlementTicketEvent,
    StackTrySettleEvent, TaskDeprecationEvent, TaskRegisteredEvent,
};
use chrono::{DateTime, Utc};
use tracing::{info, instrument, trace};

use crate::{
    state_manager::Result,
    timestamp_to_datetime_or_now,
    types::{AtomaAtomaStateManagerEvent, Stack},
    AtomaStateManager, AtomaStateManagerError,
};

#[instrument(level = "trace", skip_all)]
pub async fn handle_atoma_event(
    event: AtomaEvent,
    state_manager: &AtomaStateManager,
) -> Result<()> {
    match event {
        AtomaEvent::TaskRegisteredEvent(event) => handle_new_task_event(state_manager, event).await,
        AtomaEvent::TaskDeprecationEvent(event) => {
            handle_task_deprecation_event(state_manager, event).await
        }
        AtomaEvent::NodeSubscribedToTaskEvent(event) => {
            handle_node_task_subscription_event(state_manager, event).await
        }
        AtomaEvent::NodeSubscriptionUpdatedEvent(event) => {
            handle_node_task_subscription_updated_event(state_manager, event).await
        }
        AtomaEvent::NodeUnsubscribedFromTaskEvent(event) => {
            handle_node_task_unsubscription_event(state_manager, event).await
        }
        AtomaEvent::StackCreatedEvent((event, timestamp)) => {
            handle_create_stack_stats(
                state_manager,
                event,
                timestamp_to_datetime_or_now(timestamp),
            )
            .await?;
            Ok(())
        }
        AtomaEvent::StackCreateAndUpdateEvent(event) => {
            // NOTE: Don't handle creation here. It's handled when the stack is created right away.
            info!("Stack creates and update event: {:?}", event);
            Ok(())
        }
        AtomaEvent::StackTrySettleEvent((event, timestamp)) => {
            handle_stack_try_settle_event(
                state_manager,
                event,
                timestamp_to_datetime_or_now(timestamp),
            )
            .await
        }
        AtomaEvent::StackSettlementTicketEvent(event) => {
            handle_stack_settlement_ticket_event(state_manager, event).await
        }
        AtomaEvent::StackSettlementTicketClaimedEvent(event) => {
            handle_stack_settlement_ticket_claimed_event(state_manager, event).await
        }
        AtomaEvent::StackAttestationDisputeEvent(event) => {
            handle_stack_attestation_dispute_event(state_manager, event).await
        }
        AtomaEvent::NewStackSettlementAttestationEvent(event) => {
            handle_new_stack_settlement_attestation_event(state_manager, event).await
        }
        AtomaEvent::NewKeyRotationEvent(event) => {
            handle_new_key_rotation_event(state_manager, event).await
        }
        AtomaEvent::NodePublicKeyCommittmentEvent(event) => {
            handle_node_key_rotation_event(state_manager, event).await
        }
        AtomaEvent::PublishedEvent(event) => {
            info!("Published event: {:?}", event);
            Ok(())
        }
        AtomaEvent::NodeRegisteredEvent((event, address)) => {
            handle_node_registration_event(state_manager, event, address.to_string()).await
        }
        AtomaEvent::NodeSubscribedToModelEvent(event) => {
            info!("Node subscribed to model event: {:?}", event);
            Ok(())
        }
        AtomaEvent::FirstSubmissionEvent(event) => {
            info!("First submission event: {:?}", event);
            Ok(())
        }
        AtomaEvent::DisputeEvent(event) => {
            info!("Dispute event: {:?}", event);
            Ok(())
        }
        AtomaEvent::NewlySampledNodesEvent(event) => {
            info!("Newly sampled nodes event: {:?}", event);
            Ok(())
        }
        AtomaEvent::SettledEvent(event) => {
            info!("Settled event: {:?}", event);
            Ok(())
        }
        AtomaEvent::RetrySettlementEvent(event) => {
            info!("Retry settlement event: {:?}", event);
            Ok(())
        }
        AtomaEvent::TaskRemovedEvent(event) => {
            info!("Task removed event: {:?}", event);
            Ok(())
        }
        AtomaEvent::Text2ImagePromptEvent(event) => {
            info!("Text2Image prompt event: {:?}", event);
            Ok(())
        }
        AtomaEvent::Text2TextPromptEvent(event) => {
            info!("Text2Text prompt event: {:?}", event);
            Ok(())
        }
    }
}

/// Handles a new task event by processing and inserting it into the database.
///
/// This function takes a serialized `TaskRegisteredEvent`, deserializes it, and
/// inserts the corresponding task into the database using the provided `AtomaStateManager`.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `value` - A `serde_json::Value` containing the serialized `TaskRegisteredEvent`.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the task was successfully processed and inserted, or an error otherwise.
///
/// # Errors
///
/// This function will return an error if:
/// * The `value` cannot be deserialized into a `TaskRegisteredEvent`.
/// * The `AtomaStateManager` fails to insert the new task into the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_new_task_event(
    state_manager: &AtomaStateManager,
    event: TaskRegisteredEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-new-task-event",
        "Processing new task event"
    );
    let task = event.into();
    state_manager.state.insert_new_task(task).await?;
    Ok(())
}

/// Handles a task deprecation event.
///
/// This function processes a task deprecation event by parsing the event data,
/// extracting the necessary information, and updating the task's status in the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `value` - A `serde_json::Value` containing the serialized task deprecation event data.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `TaskDeprecationEvent`.
/// * The database operation to deprecate the task fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Deserializes the input `value` into a `TaskDeprecationEvent`.
/// 2. Extracts the `task_small_id` and `epoch` from the event.
/// 3. Calls the `deprecate_task` method on the `AtomaStateManager` to update the task's status in the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_task_deprecation_event(
    state_manager: &AtomaStateManager,
    event: TaskDeprecationEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-task-deprecation-event",
        "Processing task deprecation event"
    );
    let task_small_id = event.task_small_id;
    let epoch = event.epoch;
    state_manager
        .state
        .deprecate_task(task_small_id.inner as i64, epoch as i64)
        .await?;
    Ok(())
}

/// Handles a node task subscription event.
///
/// This function processes a node task subscription event by parsing the event data,
/// extracting the necessary information, and updating the node's subscription to the task in the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `NodeSubscribedToTaskEvent` containing the details of the subscription event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `NodeSubscribedToTaskEvent`.
/// * The database operation to subscribe the node to the task fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `node_small_id`, `task_small_id`, `price_per_compute_unit`, and `max_num_compute_units` from the event.
/// 2. Calls the `subscribe_node_to_task` method on the `AtomaStateManager` to update the node's subscription in the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_node_task_subscription_event(
    state_manager: &AtomaStateManager,
    event: NodeSubscribedToTaskEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-node-task-subscription-event",
        "Processing node subscription event"
    );
    let node_small_id = event.node_small_id.inner as i64;
    let task_small_id = event.task_small_id.inner as i64;
    let price_per_one_million_compute_units = event.price_per_one_million_compute_units as i64;
    let max_num_compute_units = event.max_num_compute_units as i64;
    state_manager
        .state
        .subscribe_node_to_task(
            node_small_id,
            task_small_id,
            price_per_one_million_compute_units,
            max_num_compute_units,
        )
        .await?;
    Ok(())
}

/// Handles a node task subscription updated event.
///
/// This function processes a node task subscription updated event by parsing the event data,
/// extracting the necessary information, and updating the node's subscription to the task in the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `NodeSubscriptionUpdatedEvent` containing the details of the subscription update.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `NodeSubscriptionUpdatedEvent`.
/// * The database operation to update the node's subscription to the task fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `node_small_id`, `task_small_id`, `price_per_compute_unit`, and `max_num_compute_units` from the event.
/// 2. Calls the `update_node_subscription` method on the `AtomaStateManager` to update the node's subscription in the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_node_task_subscription_updated_event(
    state_manager: &AtomaStateManager,
    event: NodeSubscriptionUpdatedEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-node-task-subscription-updated-event",
        "Processing node subscription updated event"
    );
    let node_small_id = event.node_small_id.inner as i64;
    let task_small_id = event.task_small_id.inner as i64;
    let price_per_one_million_compute_units = event.price_per_one_million_compute_units as i64;
    let max_num_compute_units = event.max_num_compute_units as i64;
    state_manager
        .state
        .update_node_subscription(
            node_small_id,
            task_small_id,
            price_per_one_million_compute_units,
            max_num_compute_units,
        )
        .await?;
    Ok(())
}

/// Handles a node task unsubscription event.
///
/// This function processes a node task unsubscription event by parsing the event data,
/// extracting the necessary information, and updating the node's subscription status in the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `NodeUnsubscribedFromTaskEvent` containing the details of the unsubscription event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `NodeUnsubscribedFromTaskEvent`.
/// * The database operation to unsubscribe the node from the task fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `node_small_id` and `task_small_id` from the event.
/// 2. Calls the `unsubscribe_node_from_task` method on the `AtomaStateManager` to update the node's subscription status in the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_node_task_unsubscription_event(
    state_manager: &AtomaStateManager,
    event: NodeUnsubscribedFromTaskEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-node-task-unsubscription-event",
        "Processing node unsubscription event"
    );
    let node_small_id = event.node_small_id.inner as i64;
    let task_small_id = event.task_small_id.inner as i64;
    state_manager
        .state
        .unsubscribe_node_from_task(node_small_id, task_small_id)
        .await?;
    Ok(())
}

/// Handles a stack created event.
///
/// This function processes a stack created event by parsing the event data,
/// checking if the selected node is one of the current nodes, and if so,
/// inserting the new stack into the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `StackCreatedEvent` containing the details of the stack creation event.
/// * `node_small_ids` - A slice of `u64` values representing the small IDs of the current nodes.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `StackCreatedEvent`.
/// * The database operation to insert the new stack fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `selected_node_id` from the event.
/// 2. Checks if the `selected_node_id` is present in the `node_small_ids` slice.
/// 3. If the node is valid, it converts the event into a stack object and inserts it into the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_stack_created_event(
    state_manager: &AtomaStateManager,
    event: StackCreatedEvent,
    already_computed_units: i64,
    user_id: i64,
) -> Result<()> {
    let node_small_id = event.selected_node_id.inner;
    trace!(
        target = "atoma-state-handlers",
        event = "handle-stack-created-event",
        "Stack selected current node, with id {node_small_id}, inserting new stack"
    );
    let mut stack: Stack = event.into();
    stack.already_computed_units = already_computed_units;
    state_manager.state.insert_new_stack(stack, user_id).await?;
    Ok(())
}

/// Handles create stack for stats.
///
/// This function processes a stack created event by parsing the event data,
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `StackCreatedEvent` containing the details of the stack creation event.
/// * `timestamp` - The timestamp of the event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `StackCreatedEvent`.
/// * The database operation to insert the new stack fails.
///
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_create_stack_stats(
    state_manager: &AtomaStateManager,
    event: StackCreatedEvent,
    timestamp: DateTime<Utc>,
) -> Result<()> {
    let stack = event.into();
    state_manager
        .state
        .new_stats_stack(stack, timestamp)
        .await?;
    Ok(())
}

/// Handles a stack try settle event.
///
/// This function processes a stack try settle event by parsing the event data,
/// converting it into a stack settlement ticket, and inserting it into the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `StackTrySettleEvent` containing the details of the stack try settle event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `StackTrySettleEvent`.
/// * The database operation to insert the new stack settlement ticket fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Converts the `StackTrySettleEvent` into a stack settlement ticket.
/// 2. Calls the `insert_new_stack_settlement_ticket` method on the `AtomaStateManager` to insert the ticket into the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_stack_try_settle_event(
    state_manager: &AtomaStateManager,
    event: StackTrySettleEvent,
    timestamp: DateTime<Utc>,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-stack-try-settle-event",
        "Processing stack try settle event"
    );
    let stack_settlement_ticket = event.into();
    state_manager
        .state
        .insert_new_stack_settlement_ticket(stack_settlement_ticket, timestamp)
        .await?;
    Ok(())
}

/// Handles a new stack settlement attestation event.
///
/// This function processes a new stack settlement attestation event by parsing the event data
/// and updating the corresponding stack settlement ticket in the database with attestation commitments.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `NewStackSettlementAttestationEvent` containing the details of the attestation event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `NewStackSettlementAttestationEvent`.
/// * The database operation to update the stack settlement ticket with attestation commitments fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `stack_small_id`, `attestation_node_id`, `committed_stack_proof`, and `stack_merkle_leaf` from the event.
/// 2. Calls the `update_stack_settlement_ticket_with_attestation_commitments` method on the `AtomaStateManager` to update the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_new_stack_settlement_attestation_event(
    state_manager: &AtomaStateManager,
    event: NewStackSettlementAttestationEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-new-stack-settlement-attestation-event",
        "Processing new stack settlement attestation event"
    );
    let stack_small_id = event.stack_small_id.inner as i64;
    let attestation_node_id = event.attestation_node_id.inner as i64;
    let committed_stack_proof = event.committed_stack_proof;
    let stack_merkle_leaf = event.stack_merkle_leaf;

    state_manager
        .state
        .update_stack_settlement_ticket_with_attestation_commitments(
            stack_small_id,
            committed_stack_proof,
            stack_merkle_leaf,
            attestation_node_id,
        )
        .await?;
    Ok(())
}

/// Handles a stack settlement ticket event.
///
/// This function processes a stack settlement ticket event by parsing the event data
/// and updating the corresponding stack settlement ticket in the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `StackSettlementTicketEvent` containing the details of the stack settlement ticket event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `StackSettlementTicketEvent`.
/// * The database operation to settle the stack settlement ticket fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `stack_small_id` and `dispute_settled_at_epoch` from the event.
/// 2. Calls the `settle_stack_settlement_ticket` method on the `AtomaStateManager` to update the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_stack_settlement_ticket_event(
    state_manager: &AtomaStateManager,
    event: StackSettlementTicketEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-stack-settlement-ticket-event",
        "Processing stack settlement ticket event"
    );
    let stack_small_id = event.stack_small_id.inner as i64;
    let dispute_settled_at_epoch = event.dispute_settled_at_epoch as i64;
    state_manager
        .state
        .settle_stack_settlement_ticket(stack_small_id, dispute_settled_at_epoch)
        .await?;
    Ok(())
}

/// Handles a stack settlement ticket claimed event.
///
/// This function processes a stack settlement ticket claimed event by parsing the event data
/// and updating the corresponding stack settlement ticket in the database with claim information.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `StackSettlementTicketClaimedEvent` containing the details of the stack settlement ticket claimed event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `StackSettlementTicketClaimedEvent`.
/// * The database operation to update the stack settlement ticket with claim information fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `stack_small_id` and `user_refund_amount` from the event.
/// 2. Calls the `update_stack_settlement_ticket_with_claim` method on the `AtomaStateManager` to update the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_stack_settlement_ticket_claimed_event(
    state_manager: &AtomaStateManager,
    event: StackSettlementTicketClaimedEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-stack-settlement-ticket-claimed-event",
        "Processing stack settlement ticket claimed event"
    );
    let stack_small_id = event.stack_small_id.inner as i64;
    let user_refund_amount = event.user_refund_amount as i64;
    state_manager
        .state
        .update_stack_settlement_ticket_with_claim(stack_small_id, user_refund_amount)
        .await?;
    Ok(())
}

/// Handles a stack attestation dispute event.
///
/// This function processes a stack attestation dispute event by parsing the event data
/// and inserting the dispute information into the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `StackAttestationDisputeEvent` containing the details of the dispute event.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `StackAttestationDisputeEvent`.
/// * The database operation to insert the stack attestation dispute fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Converts the `StackAttestationDisputeEvent` into a stack attestation dispute object.
/// 2. Calls the `insert_stack_attestation_dispute` method on the `AtomaStateManager` to insert the dispute into the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_stack_attestation_dispute_event(
    state_manager: &AtomaStateManager,
    event: StackAttestationDisputeEvent,
) -> Result<()> {
    trace!(
        target = "atoma-state-handlers",
        event = "handle-stack-attestation-dispute-event",
        "Processing stack attestation dispute event"
    );
    let stack_attestation_dispute = event.into();
    state_manager
        .state
        .insert_stack_attestation_dispute(stack_attestation_dispute)
        .await?;
    Ok(())
}

/// Handles node registration event.
///
/// This function processes a node registration event by parsing the event data
/// and inserting the node into the database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - A `NodeRegisteredEvent` containing the details of the node registration event.
/// * `address` - The public address of the node.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function will return an error if:
/// * The event data cannot be deserialized into a `NodeRegisteredEvent`.
/// * The database operation to insert the node fails.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Extracts the `node_small_id` from the event.
/// 2. Calls the `insert_new_node` method on the `AtomaStateManager` to insert the node into the database.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_node_registration_event(
    state_manager: &AtomaStateManager,
    event: NodeRegisteredEvent,
    address: String,
) -> Result<()> {
    state_manager
        .state
        .insert_new_node(event.node_small_id.inner as i64, address)
        .await?;
    Ok(())
}

/// Handles events related to the state manager.
///
/// This function processes various events that are sent to the state manager,
/// including requests to get available stacks with compute units, update the number
/// of tokens for a stack, and update the total hash of a stack.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations.
/// * `event` - An `AtomaAtomaStateManagerEvent` enum that specifies the type of event to handle.
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the event was processed successfully, or an error if something went wrong.
///
/// # Errors
///
/// This function may return an error if:
/// * The database operations for updating tokens or hashes fail.
/// * The result sender fails to send the result for the `GetAvailableStackWithComputeUnits` event.
///
/// # Behavior
///
/// The function performs the following steps:
/// 1. Matches the incoming event to determine the type of operation to perform.
/// 2. For `GetAvailableStackWithComputeUnits`, it retrieves the available stack and sends the result.
/// 3. For `UpdateStackNumTokens`, it updates the number of tokens for the specified stack.
/// 4. For `UpdateStackTotalHash`, it updates the total hash for the specified stack.
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_state_manager_event(
    state_manager: &AtomaStateManager,
    event: AtomaAtomaStateManagerEvent,
) -> Result<()> {
    match event {
        AtomaAtomaStateManagerEvent::GetAvailableStackWithComputeUnits {
            stack_small_id,
            public_key,
            total_num_tokens,
            result_sender,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Getting available stack with compute units for stack with id: {}",
                stack_small_id
            );
            let result = state_manager
                .state
                .get_available_stack_with_compute_units(
                    stack_small_id,
                    &public_key,
                    total_num_tokens,
                )
                .await;
            result_sender
                .send(result)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::UpdateStackNumTokens {
            stack_small_id,
            estimated_total_tokens,
            total_tokens,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Updating stack num tokens for stack with id: {}",
                stack_small_id
            );
            state_manager
                .state
                .update_stack_num_tokens(stack_small_id, estimated_total_tokens, total_tokens)
                .await?;
        }
        AtomaAtomaStateManagerEvent::UpdateStackTotalHash {
            stack_small_id,
            total_hash,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Updating stack total hash for stack with id: {}",
                stack_small_id
            );
            state_manager
                .state
                .update_stack_total_hash(stack_small_id, total_hash)
                .await?;
        }
        AtomaAtomaStateManagerEvent::GetStacksForModel {
            model,
            free_compute_units,
            user_id,
            result_sender,
            is_confidential,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Getting stacks for model: {} with free compute units: {}",
                model,
                free_compute_units
            );
            let stack = state_manager
                .state
                .get_stacks_for_model(&model, free_compute_units, user_id, is_confidential)
                .await;
            result_sender
                .send(stack)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::GetTasksForModel {
            model,
            result_sender,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Getting tasks for model: {}",
                model
            );
            let tasks = state_manager.state.get_tasks_for_model(&model).await;
            result_sender
                .send(tasks)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::GetCheapestNodeForModel {
            model,
            is_confidential,
            result_sender,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Getting cheapest node for model: {}",
                model
            );
            let node = state_manager
                .state
                .get_cheapest_node_for_model(&model, is_confidential)
                .await;
            result_sender
                .send(node)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::SelectNodePublicKeyForEncryption {
            model,
            max_num_tokens,
            result_sender,
        } => {
            let node = state_manager
                .state
                .select_node_public_key_for_encryption(&model, max_num_tokens)
                .await?;
            result_sender
                .send(node)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::SelectNodePublicKeyForEncryptionForNode {
            node_small_id,
            result_sender,
        } => {
            let node = state_manager
                .state
                .select_node_public_key_for_encryption_for_node(node_small_id)
                .await?;
            result_sender
                .send(node)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::UpsertNodePublicAddress {
            node_small_id,
            public_address,
            country,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Upserting public address/country for node with id: {}",
                node_small_id
            );
            state_manager
                .state
                .update_node_public_address(node_small_id, public_address, country)
                .await?;
        }
        AtomaAtomaStateManagerEvent::GetNodePublicAddress {
            node_small_id,
            result_sender,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Getting public address for node with id: {}",
                node_small_id
            );
            let public_address = state_manager
                .state
                .get_node_public_address(node_small_id)
                .await;
            result_sender
                .send(public_address)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::GetNodeSuiAddress {
            node_small_id,
            result_sender,
        } => {
            trace!(
                target = "atoma-state-handlers",
                event = "handle-state-manager-event",
                "Getting sui address for node with id: {}",
                node_small_id
            );
            let sui_address = state_manager
                .state
                .get_node_sui_address(node_small_id)
                .await;
            result_sender
                .send(sui_address)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::NewStackAcquired {
            event,
            already_computed_units,
            transaction_timestamp: _,
            user_id,
        } => {
            handle_stack_created_event(state_manager, event, already_computed_units, user_id)
                .await?;
        }
        AtomaAtomaStateManagerEvent::UpdateNodeThroughputPerformance {
            timestamp,
            model_name,
            node_small_id,
            input_tokens,
            output_tokens,
            time,
        } => {
            state_manager
                .state
                .update_node_throughput_performance(
                    node_small_id,
                    input_tokens,
                    output_tokens,
                    time,
                )
                .await?;
            state_manager
                .state
                .add_compute_units_processed(
                    timestamp,
                    model_name,
                    input_tokens + output_tokens,
                    time,
                )
                .await?;
        }
        AtomaAtomaStateManagerEvent::UpdateNodePrefillPerformance {
            node_small_id,
            tokens,
            time,
        } => {
            state_manager
                .state
                .update_node_prefill_performance(node_small_id, tokens, time)
                .await?;
        }
        AtomaAtomaStateManagerEvent::UpdateNodeDecodePerformance {
            node_small_id,
            tokens,
            time,
        } => {
            state_manager
                .state
                .update_node_decode_performance(node_small_id, tokens, time)
                .await?;
        }
        AtomaAtomaStateManagerEvent::UpdateNodeLatencyPerformance {
            timestamp,
            node_small_id,
            latency,
        } => {
            state_manager
                .state
                .update_node_latency_performance(node_small_id, latency)
                .await?;
            state_manager.state.add_latency(timestamp, latency).await?;
        }
        AtomaAtomaStateManagerEvent::GetSelectedNodeX25519PublicKey {
            selected_node_id,
            result_sender,
        } => {
            let public_key = state_manager
                .state
                .get_selected_node_x25519_public_key(selected_node_id)
                .await;
            result_sender
                .send(public_key)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::GetUserIdByUsernamePassword {
            username,
            password,
            result_sender,
        } => {
            let user_id = state_manager
                .state
                .get_user_id_by_username_password(&username, &password)
                .await;
            result_sender
                .send(user_id)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::RegisterUserWithPassword {
            username,
            password,
            result_sender,
        } => {
            let user_id = state_manager.state.register(&username, &password).await;
            result_sender
                .send(user_id)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::IsRefreshTokenValid {
            user_id,
            refresh_token_hash,
            result_sender,
        } => {
            let is_valid = state_manager
                .state
                .is_refresh_token_valid(user_id, &refresh_token_hash)
                .await;
            result_sender
                .send(is_valid)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::StoreRefreshToken {
            user_id,
            refresh_token_hash,
        } => {
            state_manager
                .state
                .store_refresh_token(user_id, &refresh_token_hash)
                .await?;
        }
        AtomaAtomaStateManagerEvent::RevokeRefreshToken {
            user_id,
            refresh_token_hash,
        } => {
            state_manager
                .state
                .delete_refresh_token(user_id, &refresh_token_hash)
                .await?;
        }
        AtomaAtomaStateManagerEvent::IsApiTokenValid {
            api_token,
            result_sender,
        } => {
            let user_id = state_manager.state.is_api_token_valid(&api_token).await;
            result_sender
                .send(user_id)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::StoreNewApiToken { user_id, api_token } => {
            state_manager
                .state
                .store_api_token(user_id, &api_token)
                .await?;
        }
        AtomaAtomaStateManagerEvent::RevokeApiToken { user_id, api_token } => {
            state_manager
                .state
                .delete_api_token(user_id, &api_token)
                .await?;
        }
        AtomaAtomaStateManagerEvent::GetApiTokensForUser {
            user_id,
            result_sender,
        } => {
            let api_tokens = state_manager.state.get_api_tokens_for_user(user_id).await;
            result_sender
                .send(api_tokens)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::UpdateSuiAddress {
            user_id,
            sui_address,
        } => {
            state_manager
                .state
                .update_sui_address(user_id, sui_address)
                .await?;
        }
        AtomaAtomaStateManagerEvent::GetSuiAddress {
            user_id,
            result_sender,
        } => {
            let sui_address = state_manager.state.get_sui_address(user_id).await;
            result_sender
                .send(sui_address)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::GetUserId {
            sui_address,
            result_sender,
        } => {
            let user_id = state_manager.state.get_user_id(sui_address).await;
            result_sender
                .send(user_id)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
        AtomaAtomaStateManagerEvent::TopUpBalance {
            user_id,
            amount,
            timestamp,
        } => {
            state_manager
                .state
                .top_up_balance(user_id, amount, timestamp)
                .await?;
        }
        AtomaAtomaStateManagerEvent::WithdrawBalance {
            user_id,
            amount,
            result_sender,
        } => {
            let success = state_manager.state.withdraw_balance(user_id, amount).await;
            result_sender
                .send(success)
                .map_err(|_| AtomaStateManagerError::ChannelSendError)?;
        }
    }
    Ok(())
}

/// Handles a new key rotation event by updating the key rotation state in the database.
///
/// This function processes a key rotation event that occurs when the system performs a periodic
/// key rotation. It extracts the epoch and key rotation counter from the event and updates
/// the corresponding values in the state database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations
/// * `event` - A `NewKeyRotationEvent` containing:
///   * `epoch` - The epoch number when the key rotation occurred
///   * `key_rotation_counter` - The counter tracking the number of key rotations
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the key rotation was processed successfully, or an error if something went wrong
///
/// # Errors
///
/// This function will return an error if:
/// * The database operation to update the key rotation state fails
///
/// # Example
///
/// ```rust,ignore
/// use atoma_state::AtomaStateManager;
/// use atoma_sui::events::NewKeyRotationEvent;
///
/// async fn rotate_key(state_manager: &AtomaStateManager, event: NewKeyRotationEvent) {
///     if let Err(e) = handle_new_key_rotation_event(state_manager, event).await {
///         eprintln!("Failed to handle key rotation: {}", e);
///     }
/// }
/// ```
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_new_key_rotation_event(
    state_manager: &AtomaStateManager,
    event: NewKeyRotationEvent,
) -> Result<()> {
    let NewKeyRotationEvent {
        epoch,
        key_rotation_counter,
    } = event;
    state_manager
        .state
        .insert_new_key_rotation(epoch as i64, key_rotation_counter as i64)
        .await?;
    Ok(())
}

/// Handles a node key rotation event by updating the node's public key in the database.
///
/// This function processes a node key rotation event, which occurs when a node updates its
/// cryptographic keys. It extracts the relevant information from the event and updates
/// the node's public key and associated data in the state database.
///
/// # Arguments
///
/// * `state_manager` - A reference to the `AtomaStateManager` for database operations
/// * `event` - A `NodeKeyRotationEvent` containing the details of the key rotation:
///   * `epoch` - The epoch number when the key rotation occurred
///   * `node_id` - The identifier of the node performing the key rotation
///   * `node_badge_id` - The badge identifier associated with the node
///   * `new_public_key` - The new public key for the node
///   * `tee_remote_attestation_bytes` - Remote attestation data for trusted execution environment
///
/// # Returns
///
/// * `Result<()>` - Ok(()) if the key rotation was processed successfully, or an error if something went wrong
///
/// # Errors
///
/// This function will return an error if:
/// * The database operation to update the node's public key fails
///
/// # Example
///
/// ```rust,ignore
/// use atoma_state::AtomaStateManager;
/// use atoma_sui::events::NodeKeyRotationEvent;
///
/// async fn rotate_key(state_manager: &AtomaStateManager, event: NodeKeyRotationEvent) {
///     if let Err(e) = handle_node_key_rotation_event(state_manager, event).await {
///         eprintln!("Failed to handle key rotation: {}", e);
///     }
/// }
/// ```
#[instrument(level = "trace", skip_all)]
pub(crate) async fn handle_node_key_rotation_event(
    state_manager: &AtomaStateManager,
    event: NodePublicKeyCommittmentEvent,
) -> Result<()> {
    info!("Node key rotation event: {:?}", event);
    let NodePublicKeyCommittmentEvent {
        epoch,
        key_rotation_counter,
        node_id,
        new_public_key,
        tee_remote_attestation_bytes,
    } = event;
    let is_valid =
        utils::verify_quote_v4_attestation(&tee_remote_attestation_bytes, &new_public_key)
            .await
            .is_ok();
    state_manager
        .state
        .update_node_public_key(
            node_id.inner as i64,
            epoch as i64,
            key_rotation_counter as i64,
            new_public_key,
            tee_remote_attestation_bytes,
            is_valid,
        )
        .await?;
    Ok(())
}

mod utils {
    use super::*;

    use dcap_qvl::collateral::get_collateral;
    use dcap_qvl::quote::{Quote, Report};
    use dcap_qvl::verify::verify;
    use std::time::Duration;

    /// The timeout to use for quote verification.
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// The TCB update mode to use for quote verification.
    const TCB_UPDATE_MODE: &str = "early";

    /// Verifies a TEE (Trusted Execution Environment) remote attestation quote using Intel's DCAP Quote Verification Library.
    ///
    /// This function performs verification of a Quote V4 attestation by:
    /// 1. Retrieving collateral data from Intel's Provisioning Certificate Caching Service (PCCS)
    /// 2. Verifying the quote against the collateral using the current timestamp
    ///
    /// # Arguments
    ///
    /// * `tee_remote_attestation_bytes` - A byte slice containing the TEE remote attestation quote data
    /// * `new_public_key` - A byte slice containing the public key to be verified (currently unused in verification)
    ///
    /// # Returns
    ///
    /// * `Result<()>` - Ok(()) if verification succeeds, or an error if verification fails
    ///
    /// # Errors
    ///
    /// This function will return an error in the following cases:
    /// * If collateral retrieval from PCCS fails
    /// * If the system time cannot be determined
    /// * If quote verification fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use your_crate::verify_quote_v4_attestation;
    ///
    /// async fn verify_attestation() {
    ///     let quote_data = vec![/* quote data */];
    ///     let public_key = vec![/* public key data */];
    ///     
    ///     match verify_quote_v4_attestation(&quote_data, &public_key).await {
    ///         Ok(()) => println!("Attestation verified successfully"),
    ///         Err(e) => eprintln!("Attestation verification failed: {:?}", e),
    ///     }
    /// }
    /// ```
    ///
    /// # Notes
    ///
    /// * Uses Intel's PCCS service at a hardcoded URL with a 10-second timeout
    /// * The `new_public_key` parameter is currently passed through but not used in the verification process
    /// * This function is specifically for Quote V4 format attestations
    pub(crate) async fn verify_quote_v4_attestation(
        quote_bytes: &[u8],
        new_public_key: &[u8],
    ) -> Result<()> {
        let quote = Quote::parse(quote_bytes)
            .map_err(|e| AtomaStateManagerError::FailedToParseQuote(format!("{e:?}")))?;
        let fmspc = quote
            .fmspc()
            .map_err(|e| AtomaStateManagerError::FailedToRetrieveFmspc(format!("{e:?}")))?;
        let certification_tcb_url = format!(
            "https://api.trustedservices.intel.com/tdx/certification/v4/tcb?fmspc={:?}&update={TCB_UPDATE_MODE}",
            fmspc
        );
        let collateral = get_collateral(&certification_tcb_url, quote_bytes, TIMEOUT)
            .await
            .map_err(|e| AtomaStateManagerError::FailedToRetrieveCollateral(format!("{e:?}")))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| AtomaStateManagerError::UnixTimeWentBackwards(e.to_string()))?
            .as_secs();
        match quote.report {
            Report::SgxEnclave(_) => {
                return Err(AtomaStateManagerError::FailedToVerifyQuote(
                    "Report SGX type not supported".to_string(),
                ));
            }
            Report::TD10(report) => {
                if report.report_data != new_public_key {
                    return Err(AtomaStateManagerError::FailedToVerifyQuote(
                        "Report TD10 data does not match new public key".to_string(),
                    ));
                }
            }
            Report::TD15(report) => {
                if report.base.report_data != new_public_key {
                    return Err(AtomaStateManagerError::FailedToVerifyQuote(
                        "Report TD15 data does not match new public key".to_string(),
                    ));
                }
            }
        }
        verify(quote_bytes, &collateral, now)
            .map_err(|e| AtomaStateManagerError::FailedToVerifyQuote(format!("{e:?}")))?;
        Ok(())
    }
}
