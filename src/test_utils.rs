use biodivine_lib_bdd::Bdd;
use num_bigint::BigInt;
use std::fmt::Debug;

use crate::utils::{count_states_exact, encode_state_map, pick_state_map};

use crate::xml_parsing::utils::find_start_of;

use crate::symbolic_domains::symbolic_domain::{
    BinaryIntegerDomain, GrayCodeIntegerDomain, PetriNetIntegerDomain, SymbolicDomainOrd,
    UnaryIntegerDomain,
};

use crate::update::update_fn::SmartSystemUpdateFn;

pub struct ComputationStep {
    steps: usize,
    universe_unary: Bdd,
    universe_binary: Bdd,
    universe_gray: Bdd,
    universe_petri_net: Bdd,
    result_unary: Option<Bdd>,
    result_binary: Option<Bdd>,
    result_gray: Option<Bdd>,
    result_petri_net: Option<Bdd>,
    system_unary: SmartSystemUpdateFn<UnaryIntegerDomain, u8>,
    system_binary: SmartSystemUpdateFn<BinaryIntegerDomain<u8>, u8>,
    system_gray: SmartSystemUpdateFn<GrayCodeIntegerDomain<u8>, u8>,
    system_petri_net: SmartSystemUpdateFn<PetriNetIntegerDomain, u8>,
}

/// Perform one step of backward reachability procedure. Returns either a new [Bdd] value, or
/// `None` if no new predecessors can be included.
fn bwd_step<D: SymbolicDomainOrd<u8> + Debug>(
    system: &SmartSystemUpdateFn<D, u8>,
    set: &Bdd,
) -> Option<Bdd> {
    let sorted_variables = system.get_system_variables();

    for var in sorted_variables.iter().rev() {
        let predecessors = system.predecessors_async(var.as_str(), set.to_owned());

        // Should be equivalent to "predecessors \not\subseteq result".
        if !predecessors.imp(set).is_true() {
            let result = predecessors.or(set);
            return Some(result);
        }
    }

    None
}

/// The same as [bwd_step], but goes forwards, not backward.
fn fwd_step<D: SymbolicDomainOrd<u8> + Debug>(
    system: &SmartSystemUpdateFn<D, u8>,
    set: &Bdd,
) -> Option<Bdd> {
    let sorted_variables = system.get_system_variables();

    for var in sorted_variables.iter().rev() {
        let successors = system.successors_async(var.as_str(), set);

        // Should be equivalent to "predecessors \not\subseteq result".
        if !successors.imp(set).is_true() {
            let result = successors.or(set);
            return Some(result);
        }
    }

    None
}

/// A generic function that builds [SmartSystemUpdateFn] from an SBML file.
fn build_update_fn<D: SymbolicDomainOrd<u8> + Debug>(
    sbml_path: &str,
) -> SmartSystemUpdateFn<D, u8> {
    let file = std::fs::File::open(sbml_path).expect("Cannot open SBML file.");
    let reader = std::io::BufReader::new(file);
    let mut xml = xml::reader::EventReader::new(reader);

    find_start_of(&mut xml, "listOfTransitions")
        .expect("Cannot find transitions in the SBML file.");

    SmartSystemUpdateFn::try_from_xml(&mut xml).expect("Loading system fn update failed.")
}

impl ComputationStep {
    pub fn new(sbml_path: &str) -> ComputationStep {
        let system_unary = build_update_fn::<UnaryIntegerDomain>(sbml_path);
        let system_binary = build_update_fn::<BinaryIntegerDomain<u8>>(sbml_path);
        let system_gray = build_update_fn::<GrayCodeIntegerDomain<u8>>(sbml_path);
        let system_petri_net = build_update_fn::<PetriNetIntegerDomain>(sbml_path);

        ComputationStep {
            steps: 0,
            result_unary: None,
            result_binary: None,
            result_gray: None,
            result_petri_net: None,
            universe_unary: system_unary.unit_vertex_set(),
            universe_binary: system_binary.unit_vertex_set(),
            universe_gray: system_gray.unit_vertex_set(),
            universe_petri_net: system_petri_net.unit_vertex_set(),
            system_unary,
            system_binary,
            system_gray,
            system_petri_net,
        }
    }

    /// True if the computation explored all states of the system.
    pub fn is_done(&self) -> bool {
        self.universe_unary.is_false()
    }

    pub fn can_initialize(&self) -> bool {
        self.result_unary.is_none()
    }

    pub fn remaining(&self) -> BigInt {
        count_states_exact(&self.system_unary, &self.universe_unary)
    }

    /// Setup a new initial state from the remaining universe of states. The intermediate result
    /// must be `None` and the computation must not be done (see [ComputationStep::is_done]).
    pub fn initialize(&mut self) {
        assert!(!self.is_done());
        assert!(self.can_initialize());
        let state = pick_state_map::<UnaryIntegerDomain>(&self.system_unary, &self.universe_unary);
        self.steps = 0;
        self.result_unary = Some(encode_state_map(&self.system_unary, &state));
        self.result_binary = Some(encode_state_map(&self.system_binary, &state));
        self.result_gray = Some(encode_state_map(&self.system_gray, &state));
        self.result_petri_net = Some(encode_state_map(&self.system_petri_net, &state));
    }

    pub fn perform_bwd_step(&mut self) {
        self.steps += 1;
        self.result_unary = bwd_step(&self.system_unary, self.result_unary.as_ref().unwrap());
        self.result_binary = bwd_step(&self.system_binary, self.result_binary.as_ref().unwrap());
        self.result_gray = bwd_step(&self.system_gray, self.result_gray.as_ref().unwrap());
        self.result_petri_net = bwd_step(
            &self.system_petri_net,
            self.result_petri_net.as_ref().unwrap(),
        );
        if let Some(result_unary) = &self.result_unary {
            self.universe_unary = self.universe_unary.and_not(result_unary);
        }
        if let Some(result_binary) = &self.result_binary {
            self.universe_binary = self.universe_binary.and_not(result_binary);
        }
        if let Some(result_gray) = &self.result_gray {
            self.universe_gray = self.universe_gray.and_not(result_gray);
        }
        if let Some(result_petri_net) = &self.result_petri_net {
            self.universe_petri_net = self.universe_petri_net.and_not(result_petri_net);
        }
    }

    pub fn perform_fwd_step(&mut self) {
        self.steps += 1;
        self.result_unary = fwd_step(&self.system_unary, self.result_unary.as_ref().unwrap());
        self.result_binary = fwd_step(&self.system_binary, self.result_binary.as_ref().unwrap());
        self.result_gray = fwd_step(&self.system_gray, self.result_gray.as_ref().unwrap());
        self.result_petri_net = fwd_step(
            &self.system_petri_net,
            self.result_petri_net.as_ref().unwrap(),
        );
        if let Some(result_unary) = &self.result_unary {
            self.universe_unary = self.universe_unary.and_not(result_unary);
        }
        if let Some(result_binary) = &self.result_binary {
            self.universe_binary = self.universe_binary.and_not(result_binary);
        }
        if let Some(result_gray) = &self.result_gray {
            self.universe_gray = self.universe_gray.and_not(result_gray);
        }
        if let Some(result_petri_net) = &self.result_petri_net {
            self.universe_petri_net = self.universe_petri_net.and_not(result_petri_net);
        }
    }

    pub fn check_consistency(&self) {
        let count_unary = self
            .result_unary
            .as_ref()
            .map(|it| count_states_exact(&self.system_unary, it));
        let count_binary = self
            .result_binary
            .as_ref()
            .map(|it| count_states_exact(&self.system_binary, it));
        let count_gray = self
            .result_gray
            .as_ref()
            .map(|it| count_states_exact(&self.system_gray, it));
        let count_petri_net = self
            .result_petri_net
            .as_ref()
            .map(|it| count_states_exact(&self.system_petri_net, it));
        if count_unary != count_binary
            || count_binary != count_gray
            || count_gray != count_petri_net
        {
            panic!(
                "Error at step {}. {:?} <> {:?} <> {:?} <> {:?}",
                self.steps, count_unary, count_binary, count_gray, count_petri_net
            )
        } else {
            println!(
                "Step {} successful. Current result state count: {:?}",
                self.steps, count_unary
            );
            println!(
                " > BDD sizes: {:?} {:?} {:?} {:?}",
                self.result_unary.as_ref().map(|it| it.size()),
                self.result_binary.as_ref().map(|it| it.size()),
                self.result_gray.as_ref().map(|it| it.size()),
                self.result_petri_net.as_ref().map(|it| it.size()),
            );
        }
    }
}
