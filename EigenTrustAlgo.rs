use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{AccountInfo, next_account_info},
    entrypoint,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::{clock::Clock, Sysvar},
};
use std::collections::HashMap;

// Define the state of a car's trust
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct CarTrustState {
    pub trust_score: f64,
    pub last_updated_timestamp: i64,
}

// Define the instruction data
#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub enum TrustInstruction {
    InitializeCar {
        initial_trust: f64,
    },
    ReportMessageOutcome {
        reporter: Pubkey,
        message_sender: Pubkey,
        is_true: bool,
    },
    UpdateTrustScores,
}

// Program entrypoint
entrypoint!(process_instruction);

// Function to process instructions
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = TrustInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    match instruction {
        TrustInstruction::InitializeCar { initial_trust } => {
            msg!("Instruction: InitializeCar");
            initialize_car(program_id, accounts, initial_trust)?;
        }
        TrustInstruction::ReportMessageOutcome {
            reporter,
            message_sender,
            is_true,
        } => {
            msg!("Instruction: ReportMessageOutcome");
            report_message_outcome(program_id, accounts, reporter, message_sender, is_true)?;
        }
        TrustInstruction::UpdateTrustScores => {
            msg!("Instruction: UpdateTrustScores");
            update_trust_scores(program_id, accounts)?;
        }
    }

    Ok(())
}

// Function to initialize a car's trust state
fn initialize_car(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    initial_trust: f64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let car_account = next_account_info(accounts_iter)?;
    let payer_account = next_account_info(accounts_iter)?;
    let system_program_account = next_account_info(accounts_iter)?;
    let clock = Clock::get()?;

    if car_account.owner != program_id {
        return Err(ProgramError::IncorrectProgramId);
    }

    if car_account.data_len() > 0 {
        msg!("Car account already initialized");
        return Ok(());
    }

    let trust_state = CarTrustState {
        trust_score: initial_trust,
        last_updated_timestamp: clock.unix_timestamp,
    };

    let account_span = trust_state.try_to_vec()?.len();
    let rent_lamports = solana_program::rent::Rent::get()?.minimum_balance(account_span);

    solana_program::program::invoke(
        &solana_program::system_instruction::create_account(
            payer_account.key,
            car_account.key,
            rent_lamports,
            account_span as u64,
            program_id,
        ),
        &[
            payer_account.clone(),
            car_account.clone(),
            system_program_account.clone(),
        ],
    )?;

    trust_state.serialize(&mut &mut car_account.data.borrow_mut()[..])?;

    msg!("Car {} initialized with trust score: {}", car_account.key, initial_trust);
    Ok(())
}

// Function for a car to report the outcome of a message from another car
fn report_message_outcome(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    reporter: Pubkey,
    message_sender: Pubkey,
    is_true: bool,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();

    let reporter_account_info = next_account_info(accounts_iter)?;
    let sender_account_info = next_account_info(accounts_iter)?;

    if reporter_account_info.key != &reporter || reporter_account_info.owner != program_id {
        return Err(ProgramError::IncorrectAccountOwner);
    }
    if sender_account_info.key != &message_sender || sender_account_info.owner != program_id {
        return Err(ProgramError::IncorrectAccountOwner);
    }

    let mut reporter_trust_state = CarTrustState::try_from_slice(&reporter_account_info.data.borrow())?;
    let mut sender_trust_state = CarTrustState::try_from_slice(&sender_account_info.data.borrow())?;
    let clock = Clock::get()?;

    // Simple trust update based on feedback
    let feedback_weight = 0.1;
    let time_decay_factor = 0.0001; // Reduce impact of older feedback

    let time_elapsed_reporter = (clock.unix_timestamp - reporter_trust_state.last_updated_timestamp) as f64;
    let time_elapsed_sender = (clock.unix_timestamp - sender_trust_state.last_updated_timestamp) as f64;

    let decayed_reporter_trust = reporter_trust_state.trust_score * (1.0 - time_decay_factor * time_elapsed_reporter);
    let decayed_sender_trust = sender_trust_state.trust_score * (1.0 - time_decay_factor * time_elapsed_sender);

    if is_true {
        sender_trust_state.trust_score += feedback_weight * decayed_reporter_trust * (1.0 - decayed_sender_trust);
    } else {
        sender_trust_state.trust_score -= feedback_weight * decayed_reporter_trust * decayed_sender_trust;
    }

    // Ensure trust score stays within [0, 1] (or your desired bounds)
    sender_trust_state.trust_score = sender_trust_state.trust_score.max(0.0).min(1.0);
    sender_trust_state.last_updated_timestamp = clock.unix_timestamp;

    reporter_trust_state.last_updated_timestamp = clock.unix_timestamp;

    sender_trust_state.serialize(&mut &mut sender_account_info.data.borrow_mut()[..])?;
    reporter_trust_state.serialize(&mut &mut reporter_account_info.data.borrow_mut()[..])?;

    msg!("Car {} reported message from {} as {}", reporter.to_string(), message_sender.to_string(), is_true);
    msg!("Trust score of {} updated to: {}", message_sender.to_string(), sender_trust_state.trust_score);

    Ok(())
}

// Function to periodically update trust scores based on accumulated reports
fn update_trust_scores(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    msg!("Starting trust score update...");
    let clock = Clock::get()?;
    let mut car_trust_states: HashMap<Pubkey, CarTrustState> = HashMap::new();
    let mut car_account_infos: HashMap<Pubkey, AccountInfo> = HashMap::new();

    // Collect all car trust states
    for account_info in accounts.iter() {
        if account_info.owner == program_id && account_info.data_len() > 0 {
            let trust_state = CarTrustState::try_from_slice(&account_info.data.borrow())?;
            car_trust_states.insert(*account_info.key, trust_state);
            car_account_infos.insert(*account_info.key, account_info.clone());
        }
    }

    if car_trust_states.is_empty() {
        msg!("No car trust accounts found.");
        return Ok(());
    }

    let num_cars = car_trust_states.len();
    let mut new_trust_scores: HashMap<Pubkey, f64> = HashMap::new();
    let alpha = 0.85; // Damping factor for EigenTrust

    // Simple EigenTrust-like iteration (very basic and needs refinement)
    for _iteration in 0..5 { // Number of iterations
        for (car_id, current_trust) in car_trust_states.iter() {
            let mut weighted_sum = 0.0;
            let mut total_trust_of_voters = 0.0;

            // Iterate through all other cars as potential "voters"
            for (voter_id, voter_trust) in car_trust_states.iter() {
                if voter_id != car_id {
                    // In a real EigenTrust, you'd need explicit trust relationships.
                    // Here, we're making a simplification: all cars can "vote"
                    // based on their own trust score.

                    // For simplicity, let's assume a car's trust in another is proportional to its own trust.
                    let trust_voter_in_car = voter_trust.trust_score;
                    weighted_sum += trust_voter_in_car * (if let Some(account) = car_account_infos.get(car_id) {
                        // We don't have explicit message history in this simplified example.
                        // In a real system, you'd look at past reports from 'voter_id' about 'car_id'.
                        // For this basic example, we'll just consider the current trust.
                        if current_trust.trust_score > 0.5 { 1.0 } else { 0.0 } // Placeholder for agreement
                    } else { 0.5 }); // Default if car info not found

                    total_trust_of_voters += trust_voter_in_car;
                }
            }

            let normalized_weighted_sum = if total_trust_of_voters > 0.0 {
                weighted_sum / total_trust_of_voters
            } else {
                0.5 // Default if no voters
            };

            // Apply EigenTrust update rule (simplified)
            let initial_trust = 0.6; // Base initial trust
            let new_score = alpha * normalized_weighted_sum + (1.0 - alpha) * initial_trust;
            new_trust_scores.insert(*car_id, new_score.max(0.0).min(1.0));
        }
        // Update trust states for the next iteration
        for (car_id, new_score) in new_trust_scores.iter() {
            if let Some(mut state) = car_trust_states.get_mut(car_id) {
                state.trust_score = *new_score;
                state.last_updated_timestamp = clock.unix_timestamp;
            }
        }
    }

    // Serialize and update accounts on Solana
    for (car_id, updated_state) in car_trust_states.iter() {
        if let Some(account_info) = car_account_infos.get(car_id) {
            updated_state.serialize(&mut &mut account_info.data.borrow_mut()[..])?;
            msg!("Updated trust score of {} to: {}", car_id.to_string(), updated_state.trust_score);
        }
    }

    msg!("Trust score update complete.");
    Ok(())
}

#[cfg(not(feature = "no-entrypoint"))]
#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::clock::Epoch;
    use solana_program::instruction::{AccountMeta, Instruction};
    use solana_program::program_error::Error;
    use solana_program::system_instruction;
    use solana_program::sysvar::rent::Rent;
    use std::mem;

    fn create_account_info<'a>(
        owner: &Pubkey,
        lamports: &'a mut u64,
        data: &'a mut [u8],
    ) -> AccountInfo<'a> {
        AccountInfo::new(
            &Pubkey::new_unique(),
            false,
            true,
            lamports,
            data,
            owner,
            false,
            Epoch::default(),
        )
    }

    #[test]
    fn test_initialize_car() {
        let program_id = Pubkey::new_unique();
        let mut lamports = Rent::default().minimum_balance(CarTrustState::get_packed_len());
        let mut data = vec![0u8; CarTrustState::get_packed_len()];
        let car_account = &mut create_account_info(&program_id, &mut lamports, &mut data);
        let payer_account = &mut create_account_info(&Pubkey::new_unique(), &mut 1000000000, &mut []);
        let system_program_account = &mut create_account_info(&solana_program::system_program::id(), &mut 0, &mut []);
        let accounts = &[car_account.clone(), payer_account.clone(), system_program_account.clone()];
        let initial_trust = 0.75;
        let instruction_data = TrustInstruction::InitializeCar { initial_trust }.try_to_vec().unwrap();

        let result = process_instruction(&program_id, accounts, &instruction_data);
        assert!(result.is_ok());

        let initialized_state = CarTrustState::try_from_slice(&car_account.data.borrow()).unwrap();
        assert_eq!(initialized_state.trust_score, initial_trust);
    }

    #[test]
    fn test_report_message_outcome() {
        let program_id = Pubkey::new_unique();
        let mut reporter_lamports = Rent::default().minimum_balance(CarTrustState::get_packed_len());
        let mut reporter_data = vec![0u8; CarTrustState::get_packed_len()];
        let reporter_account = &mut create_account_info(&program_id, &mut reporter_lamports, &mut reporter_data);
        let initial_reporter_trust = 0.9;
        CarTrustState {
            trust_score: initial_reporter_trust,
            last_updated_timestamp: Clock::get().unwrap().unix_timestamp,
        }
        .serialize(&mut &mut reporter_account.data.borrow_mut()[..])
        .unwrap();

        let mut sender_lamports = Rent::default().minimum_balance(CarTrustState::get_packed_len());
        let mut sender_data = vec![0u8; CarTrustState::get_packed_len()];
        let sender_account = &mut create_account_info(&program_id, &mut sender_lamports, &mut sender_data);
        let initial_sender_trust = 0.5;
        CarTrustState {
            trust_score: initial_sender_trust,
            last_updated_timestamp: Clock::get().unwrap().unix_timestamp,
        }
        .serialize(&mut &mut sender_account.data.borrow_mut()[..])
        .unwrap();

        let accounts = &[reporter_account.clone(), sender_account.clone()];
        let is_true = true;
        let instruction_data = TrustInstruction::ReportMessageOutcome {
            reporter: *reporter_account.key,
            message_sender: *sender_account.key,
            is_true,
        }
        .try_to_vec()
        .unwrap();

        let result = process_instruction(&program_id, accounts, &instruction_data);
        assert!(result.is_ok());

        let updated_sender_state = CarTrustState::try_from_slice(&sender_account.data.borrow()).unwrap();
        assert!(updated_sender_state.trust_score > initial_sender_trust);
    }

    impl CarTrustState {
        fn get_packed_len() -> usize {
            mem::size_of::<f64>() + mem::size_of::<i64>()
        }
    }
}

explain the woeking od the code
