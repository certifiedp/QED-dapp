use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use plonky2::{
    field::{extension::Extendable, goldilocks_field::GoldilocksField},
    hash::{hash_types::RichField, poseidon::PoseidonHash},
    iop::witness::PartialWitness,
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData},
        config::{AlgebraicHasher, GenericConfig, PoseidonGoldilocksConfig},
        proof::ProofWithPublicInputs,
    },
};
use plonky2_tree_hacks::{
    common::{
        hash::merkle::{
            gadgets::delta_merkle_proof::DeltaMerkleProofGadget,
            helpers::merkle_proof::DeltaMerkleProof,
        },
        u32::multiple_comparison::list_le_circuit,
        WHashOut,
    },
    utils::zmt::{
        node_store::simple_node_store::SimpleNodeStore, zero_merkle_tree::ZeroMerkleTree,
    },
};

pub struct BalanceUpdateGadget {
    pub sender_update: DeltaMerkleProofGadget,
    pub receiver_update: DeltaMerkleProofGadget,
}
pub struct BalanceUpdate<F: RichField> {
    pub sender_update: DeltaMerkleProof<F>,
    pub receiver_update: DeltaMerkleProof<F>,
}
impl BalanceUpdateGadget {
    pub fn add_virtual_to<H: AlgebraicHasher<F>, F: RichField + Extendable<D>, const D: usize>(
        builder: &mut CircuitBuilder<F, D>,
        tree_height: usize,
    ) -> Self {
        let sender_update = DeltaMerkleProofGadget::add_virtual_to::<H, F, D>(builder, tree_height);
        let receiver_update =
            DeltaMerkleProofGadget::add_virtual_to::<H, F, D>(builder, tree_height);

        let amount_recv = builder.sub(
            receiver_update.new_value.elements[0],
            receiver_update.old_value.elements[0],
        );
        let amount_send = builder.sub(
            sender_update.old_value.elements[0],
            sender_update.new_value.elements[0],
        );
        builder.connect(amount_recv, amount_send);

        let overflow_checks = list_le_circuit(
            builder,
            vec![
                receiver_update.old_value.elements[0],
                sender_update.new_value.elements[0],
            ],
            vec![
                receiver_update.new_value.elements[0],
                sender_update.old_value.elements[0],
            ],
            32,
        );
        let true_target = builder.one();
        builder.connect(overflow_checks.target, true_target);

        builder.connect_hashes(sender_update.new_root, receiver_update.old_root);
        Self {
            sender_update,
            receiver_update,
        }
    }
    pub fn set_witness_proof<F: RichField>(
        &self,
        witness: &mut PartialWitness<F>,
        input: &BalanceUpdate<F>,
    ) {
        self.sender_update
            .set_witness_proof(witness, &input.sender_update);
        self.receiver_update
            .set_witness_proof(witness, &input.receiver_update);
    }
}

pub struct UpdateBalanceCircuit<
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F> + 'static,
    const D: usize,
> where
    <C as GenericConfig<D>>::Hasher: AlgebraicHasher<F>,
{
    pub updates: Vec<BalanceUpdateGadget>,
    pub base_circuit_data: CircuitData<F, C, D>,
}

impl<F: RichField + Extendable<D>, C: GenericConfig<D, F = F> + 'static, const D: usize>
    UpdateBalanceCircuit<F, C, D>
where
    <C as GenericConfig<D>>::Hasher: AlgebraicHasher<F>,
{
    pub fn new(number_updates: usize, tree_height: usize) -> Self {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let updates: Vec<BalanceUpdateGadget> = (0..number_updates)
            .map(|_| {
                BalanceUpdateGadget::add_virtual_to::<C::Hasher, F, D>(&mut builder, tree_height)
            })
            .collect();
        for i in 1..number_updates {
            builder.connect_hashes(
                updates[i - 1].receiver_update.new_root,
                updates[i].sender_update.old_root,
            );
        }
        builder.register_public_inputs(&updates[0].sender_update.old_root.elements);
        builder
            .register_public_inputs(&updates[updates.len() - 1].receiver_update.new_root.elements);
        let base_circuit_data = builder.build::<C>();
        Self {
            updates,
            base_circuit_data,
        }
    }
    pub fn prove(
        &self,
        proofs: &Vec<BalanceUpdate<F>>,
    ) -> anyhow::Result<ProofWithPublicInputs<F, C, D>> {
        let num_updates = self.updates.len();
        assert_eq!(proofs.len(), num_updates);
        let mut pw = PartialWitness::<F>::new();
        for i in 0..num_updates {
            self.updates[i].set_witness_proof(&mut pw, &proofs[i])
        }
        self.base_circuit_data.prove(pw)
    }
}
pub struct BalanceStorage {
    pub tree: ZeroMerkleTree<GoldilocksField, PoseidonHash, SimpleNodeStore>,
}

impl BalanceStorage {
    pub fn new(height: u8, start_balances: Vec<u32>) -> Self {
        let mut tree = ZeroMerkleTree::<GoldilocksField, PoseidonHash, SimpleNodeStore>::new(
            height,
            SimpleNodeStore::new(),
        );

        for (i, balance) in start_balances.iter().enumerate() {
            tree.set_leaf(i as u64, WHashOut::from_values((*balance) as u64, 0, 0, 0))
                .unwrap();
        }
        Self { tree }
    }
    pub fn get_balance(&self, index: u64) -> anyhow::Result<u32> {
        let balance_proof = self.tree.get_leaf(index)?;

        Ok(balance_proof.value.0.elements[0].0 as u32)
    }
    pub fn set_balance(
        &mut self,
        index: u64,
        value: u32,
    ) -> anyhow::Result<DeltaMerkleProof<GoldilocksField>> {
        let leaf_value = WHashOut::from_values(value as u64, 0, 0, 0);

        self.tree.set_leaf(index, leaf_value)
    }
    pub fn process_tx(
        &mut self,
        sender: u64,
        receiver: u64,
        amount: u32,
    ) -> anyhow::Result<BalanceUpdate<GoldilocksField>> {
        let sender_balance = self.get_balance(sender)?;
        let receiver_balance = self.get_balance(receiver)?;
        // println!("Sender balance: {}", sender_balance);
        assert!(sender_balance >= amount, "Insufficient funds");

        let sender_proof: DeltaMerkleProof<GoldilocksField> =
            self.set_balance(sender, sender_balance - amount)?;
        let receiver_proof = self.set_balance(receiver, receiver_balance + amount)?;
        // println!("New Sender balance: {}", self.get_balance(sender)?);
        Ok(BalanceUpdate {
            sender_update: sender_proof,
            receiver_update: receiver_proof,
        })
    }
    pub fn process_txs(
        &mut self,
        txs: Vec<(u64, u64, u32)>,
    ) -> anyhow::Result<Vec<BalanceUpdate<GoldilocksField>>> {
        let mut proofs = vec![];
        for (sender, receiver, amount) in txs {
            proofs.push(self.process_tx(sender, receiver, amount)?);
        }
        Ok(proofs)
    }
}

struct AppState {
    shared_map: Mutex<HashMap<Uuid, Proposal>>, // Mutex for safe concurrent access
}

pub struct Proposal {
    pub statement: String,
    pub storage: BalanceStorage,
    pub proposer_id: u32,
    pub updates: Vec<BalanceUpdate<GoldilocksField>>,
    pub is_finalized: bool,
}
impl Proposal {
    pub fn new(statement: String, proposer_id: u32) -> Self {
        // Creates a new policiy and balance storage object
        let mut start_balances = vec![0; 2];
        let updates = vec![];
        start_balances.extend(vec![1; 2_usize.pow(10)]);
        let storage = BalanceStorage::new(32, start_balances);
        let is_finalized = false;
        Self {
            statement,
            storage,
            proposer_id,
            updates,
            is_finalized,
        }
    }
}

// Votes on a specific policiy
// pub fn vote(proposal_id: u32, voter_id: u32, vote: u32) {}

// List all of the current proposals, stored in HashMap
async fn list_proposals(data: web::Data<Arc<AppState>>) -> impl Responder {
    let proposals = data.shared_map.lock().unwrap();
    let mut out = String::new();
    for (id, proposal) in proposals.iter() {
        if proposal.is_finalized {
            let no_votes = proposal.storage.get_balance(0).unwrap();
            let yes_votes = proposal.storage.get_balance(1).unwrap();
            let result = if no_votes >= yes_votes {
                "vetoed"
            } else {
                "passed"
            };
            out.push_str(&format!(
                "Proposal ID: {}, Statement: {}, Proposer ID: {}, Finalized: {}, # of Yes Votes: {}, # of No Votes: {} -> Proposal {}\n",
                id, proposal.statement, proposal.proposer_id, proposal.is_finalized, yes_votes, no_votes, result
            ));
        } else {
            out.push_str(&format!(
                "Proposal ID: {}, Statement: {}, Proposer ID: {}, Finalized: {}\n",
                id, proposal.statement, proposal.proposer_id, proposal.is_finalized
            ));
        }
    }
    HttpResponse::Ok().body(out)
}

#[derive(Deserialize)]
struct ProposeQuery {
    proposer_id: u32,
    statement: String,
}

async fn propose(data: web::Data<Arc<AppState>>, item: web::Json<ProposeQuery>) -> impl Responder {
    let mut proposals = data.shared_map.lock().unwrap();
    let new_proposal = Proposal::new(item.statement.clone(), item.proposer_id);
    let proposal_id = Uuid::new_v4();
    proposals.insert(proposal_id, new_proposal);
    HttpResponse::Ok().body(format!("New proposal {}: {}", proposal_id, item.statement))
}

#[derive(Deserialize)]
struct VoteQuery {
    proposal_id: Uuid,
    voter_id: u32,
    is_yes: bool,
}
async fn vote(data: web::Data<Arc<AppState>>, item: web::Json<VoteQuery>) -> impl Responder {
    let mut proposals = data.shared_map.lock().unwrap();
    // Moves vote from user x to 0 or 1
    // Checks if proposal exists
    let proposal = proposals.get_mut(&item.proposal_id);
    if let Some(proposal) = proposal {
        // Checks if proposal is finalized
        if proposal.is_finalized {
            return HttpResponse::BadRequest().body("Proposal is finalized");
        }
        let vote = if item.is_yes { 1 } else { 0 };
        let voter_balance = proposal.storage.get_balance(item.voter_id as u64).unwrap();
        let update = proposal
            .storage
            .process_tx(item.voter_id as u64, vote as u64, voter_balance)
            .unwrap();
        proposal.updates.push(update);
        HttpResponse::Ok().body(format!("Voted on proposal {}", item.proposal_id))
    } else {
        HttpResponse::NotFound().body("Proposal not found")
    }
}

#[derive(Deserialize)]
struct DelegateQuery {
    proposal_id: Uuid,
    voter_id: u32,
    delegator_id: u32,
}
async fn delegate(
    data: web::Data<Arc<AppState>>,
    item: web::Json<DelegateQuery>,
) -> impl Responder {
    let mut proposals = data.shared_map.lock().unwrap();
    // Delegates vote from user x to user y
    // Checks if proposal exists
    let proposal = proposals.get_mut(&item.proposal_id);
    if let Some(proposal) = proposal {
        // Checks if proposal is finalized
        if proposal.is_finalized {
            return HttpResponse::BadRequest().body("Proposal is finalized");
        }
        let voter_balance = proposal.storage.get_balance(item.voter_id as u64).unwrap();
        let update = proposal
            .storage
            .process_tx(
                item.voter_id as u64,
                item.delegator_id as u64,
                voter_balance,
            )
            .unwrap();
        proposal.updates.push(update);
        HttpResponse::Ok().body(format!("Delegated on proposal {}", item.proposal_id))
    } else {
        HttpResponse::NotFound().body("Proposal not found")
    }
}

#[derive(Deserialize)]
struct FinalizeQuery {
    proposal_id: Uuid,
    finalizer_id: u32,
}
async fn finalize(
    data: web::Data<Arc<AppState>>,
    item: web::Json<FinalizeQuery>,
) -> impl Responder {
    type F = GoldilocksField;
    type C = PoseidonGoldilocksConfig;
    const D: usize = 2;
    let mut proposals = data.shared_map.lock().unwrap();
    // Checks if proposal exists
    let proposal = proposals.get_mut(&item.proposal_id);
    if let Some(proposal) = proposal {
        // Checks if proposal is finalized
        if item.finalizer_id != proposal.proposer_id {
            return HttpResponse::BadRequest().body("Finalizer is not the proposer");
        }
        let circuit: UpdateBalanceCircuit<GoldilocksField, PoseidonGoldilocksConfig, 2> =
            UpdateBalanceCircuit::<F, C, D>::new(proposal.updates.len(), 32);
        let proof: ProofWithPublicInputs<GoldilocksField, PoseidonGoldilocksConfig, 2> =
            circuit.prove(&proposal.updates).unwrap();
        circuit.base_circuit_data.verify(proof).unwrap();
        proposal.is_finalized = true;
        let no_votes = proposal.storage.get_balance(0).unwrap();
        let yes_votes = proposal.storage.get_balance(1).unwrap();
        let result = if no_votes >= yes_votes {
            "vetoed"
        } else {
            "passed"
        };
        HttpResponse::Ok().body(format!(
            "Finalized proposal {}; # of Yes votes: {}, # of No votes: {} -> Proposal {}",
            item.proposal_id, yes_votes, no_votes, result
        ))
    } else {
        HttpResponse::NotFound().body("Proposal not found")
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let shared_state = AppState {
        shared_map: Mutex::new(HashMap::new()),
    };
    let shared_state = Arc::new(shared_state);
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(shared_state.clone()))
            .route("/", web::get().to(list_proposals))
            .route("/vote", web::post().to(vote))
            .route("/delegate", web::post().to(delegate))
            .route("/finalize", web::post().to(finalize))
            .route("/propose", web::post().to(propose))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
