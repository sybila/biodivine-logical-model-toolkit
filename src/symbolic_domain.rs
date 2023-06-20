use biodivine_lib_bdd::{
    Bdd, BddPartialValuation, BddVariable, BddVariableSet, BddVariableSetBuilder,
};
use dyn_clonable::clonable;
use std::collections::HashSet;

/// Objects implementing `SymbolicDomain` serve as encoders/decoders for their associated type
/// `T` from/into `Bdd` objects.
///
/// Note that in general, a symbolic domain tracks only a subset of all variables that can appear
/// in a BDD. Hence, the implementations should "transparently" ignore the remaining BDD variables.
///
/// We also generally *assume* that `SymbolicDomain` implements `Clone`. This makes everything
/// much easier when we start talking about composition and generally moving/sharing symbolic
/// domain objects between places. However, implementing `Clone` directly would make it impossible
/// to create `dyn SymbolicDomain` trait objects. Hence we use `dyn-cloneable` crate to generate
/// the relevant boilerplate code for us.
///
/// TODO:
///     - We might consider swapping `&T` for `T` in a few places, as the `T` type will be usually
///     quite small (maybe even `Copy`). But this might not be that important.
///     - At the moment, methods returning a `Bdd` object take a `BddVariableSet` argument.
///     In some cases, this argument is unused, but it could be needed in some more specialized
///     implementations. Later, we should try to address this in `lib-bdd` (i.e. have a better
///     API for creating BDDs without having access to the `BddVariableSet`).
#[clonable]
pub trait SymbolicDomain<T>: Clone {
    /// Encode the given `value` into the provided `BddPartialValuation`.
    ///
    /// *Contract:* This method only modifies the symbolic variables from
    /// `Self::symbolic_variables`. No other parts of the `BddPartialValuation` are affected.
    fn encode_bits(&self, bdd_valuation: &mut BddPartialValuation, value: &T);

    /// Decode a value from the provided `BddPartialValuation`.
    ///
    /// *Contract:* This method only reads the symbolic variables from `Self::symbolic_variables`.
    /// The result is undefined if `bdd_valuation` does not represent a value that is valid in
    /// the encoding implemented by this `SymbolicDomain` (i.e. if the valuation is not valid
    /// within the `Self::unit_collection` BDD object). In particular, the method can return
    /// any value or panic in such a scenario (though panics are preferred).
    fn decode_bits(&self, bdd_valuation: &BddPartialValuation) -> T;

    /// Returns the exact symbolic variables used in the encoding of this `SymbolicDomain`.
    ///
    /// *Contract:* There is no requirement for the resulting variables to be sorted in any
    /// explicit way. However, we do require that the resulting vector is the same when the method
    /// is invoked repeatedly (i.e. the order of variables cannot change non-deterministically).
    ///
    /// Furthermore, Note that not all valuations of the returned variables must encode valid
    /// values of type `T`. The actual set of all valid encoded values can be obtained
    /// through `Self::unit_collection`.
    fn symbolic_variables(&self) -> Vec<BddVariable>;

    /// Returns the number of symbolic variables used in the encoding of this symbolic domain.
    fn symbolic_size(&self) -> usize;

    /// Create a `Bdd` which represents the empty set of encoded values.
    ///
    /// Typically, this is just the `false` BDD, but some implementations may need to customize
    /// this value.
    fn empty_collection(&self, variables: &BddVariableSet) -> Bdd;

    /// Create a `Bdd` which represents all values that can be encoded by this symbolic domain.
    ///
    /// Often, this is just `true`, but may need to be customized when the size of the symbolic
    /// domain is not such that all possible encodings are used.
    fn unit_collection(&self, variables: &BddVariableSet) -> Bdd;

    /* The rest are default implementations of several utility methods. */

    /// Encode a single `value` into a `Bdd` which is satisfied for exactly this value
    /// and no other.
    ///
    /// *Contract:* The resulting BDD only uses variables from `Self::symbolic_variables`.
    fn encode_one(&self, variables: &BddVariableSet, value: &T) -> Bdd {
        let mut valuation = BddPartialValuation::empty();
        self.encode_bits(&mut valuation, value);
        variables.mk_conjunctive_clause(&valuation)
    }

    /// Interpret and decode the given `Bdd` as a single value.
    ///
    /// *Contract:* The `Bdd` object actually must be a proper encoding of a *single* value.
    /// This method should panic if the given `Bdd` is satisfied by multiple values from the
    /// symbolic domain.
    fn decode_one(&self, _variables: &BddVariableSet, value: &Bdd) -> T {
        assert!(value.is_clause());
        let clause = value.first_clause().unwrap();
        self.decode_bits(&clause)
    }

    /// Encode a collection of values into a `Bdd`.
    fn encode_collection(&self, variables: &BddVariableSet, collection: &[T]) -> Bdd {
        let clauses = collection
            .iter()
            .map(|v| {
                let mut valuation = BddPartialValuation::empty();
                self.encode_bits(&mut valuation, v);
                valuation
            })
            .collect::<Vec<_>>();

        variables.mk_dnf(&clauses)
    }

    /// Decode a collection of values stored in a `Bdd`.
    ///
    /// *Contract:* The order of returned values can be arbitrary as long as it is deterministic.
    /// Typically, this will follow some kind of implicit order enforced by the encoding.
    fn decode_collection(&self, variables: &BddVariableSet, collection: &Bdd) -> Vec<T> {
        // This cumbersome piece of code eliminates all non-encoding variables from the `collection`
        // BDD and replaces them with a value `false`. These extra `false` entries can be then
        // skipped in the final iterator over all BDD valuations.

        // At the moment, this is likely quite slow. However, if this turns out to be the final
        // design, we should be able to implement this directly in `lib-bdd` much more efficiently.
        // Furthermore, explicitly decoding the whole set of values at once is mostly a "debug"
        // operation, or performed only for very small sets. So in the end, the performance here
        // might not even matter.

        let encoding_variables = self.symbolic_variables();
        let mut ignored_variables: HashSet<BddVariable> =
            variables.variables().into_iter().collect();
        ignored_variables.retain(|x| !encoding_variables.contains(x));
        let ignored_variables: Vec<BddVariable> = ignored_variables.into_iter().collect();
        let collection = collection.exists(&ignored_variables);
        let fixed_selection = ignored_variables
            .into_iter()
            .map(|it| (it, false))
            .collect::<Vec<_>>();
        let collection = collection.select(&fixed_selection);

        let mut encoded_bits = BddPartialValuation::empty();
        collection
            .sat_valuations()
            .map(|valuation| {
                for bit in &encoding_variables {
                    encoded_bits.set_value(*bit, valuation.value(*bit))
                }
                self.decode_bits(&encoded_bits)
            })
            .collect()
    }
}

/// Implementation of a `SymbolicDomain` using unary integer encoding, i.e. each integer domain
/// `D = { 0 ... max }` is encoded using `max` symbolic variables.
///
/// In this encoding, to represent value `k \in D`, we set the values of the first `k` symbolic
/// variables to `true` and leave the remaining as `false`.
#[derive(Clone, Debug)]
pub struct UnaryIntegerDomain {
    variables: Vec<BddVariable>,
}

/*
   TODO:
       - We might want to add a proper `IntegerSymbolicDomain` trait that would have a blanket
       implementation for all SymbolicDomain<u8> types. In this trait, we would implement
       operations like "make a BDD of all values less-than-or-equal to constant X", i.e. things
       that typically appear as "atomic propositions" in update functions.
*/

impl UnaryIntegerDomain {
    /// Create a new `UnaryIntegerDomain`, such that the symbolic variables are allocated in the
    /// given `BddVariableSetBuilder`.
    pub fn new(
        builder: &mut BddVariableSetBuilder,
        name: &str,
        max_value: u8,
    ) -> UnaryIntegerDomain {
        let variables = (0..max_value)
            .map(|it| {
                let name = format!("{name}_v{}", it + 1);
                builder.make_variable(name.as_str())
            })
            .collect::<Vec<_>>();

        UnaryIntegerDomain { variables }
    }
}

impl SymbolicDomain<u8> for UnaryIntegerDomain {
    fn encode_bits(&self, bdd_valuation: &mut BddPartialValuation, value: &u8) {
        for (i, var) in self.variables.iter().enumerate() {
            bdd_valuation.set_value(*var, i < (*value as usize));
        }
    }

    fn decode_bits(&self, bdd_valuation: &BddPartialValuation) -> u8 {
        // This method does not always check if the valuation is valid in the unary encoding, it
        // just picks the "simplest" interpretation of the given valuation. For increased safety,
        // we should check that that after the last "true" value, only "false" values follow.
        let mut result = 0;
        while result < self.variables.len() {
            let variable = self.variables[result];
            // This will panic if the variable value is not provided, which is reasonable because
            // in that case, the partial valuation is not a correctly encoded value.
            if !bdd_valuation.get_value(variable).unwrap() {
                return result as u8;
            }
            result += 1;
        }
        result as u8
    }

    fn symbolic_variables(&self) -> Vec<BddVariable> {
        self.variables.clone()
    }

    fn symbolic_size(&self) -> usize {
        self.variables.len()
    }

    fn empty_collection(&self, variables: &BddVariableSet) -> Bdd {
        variables.mk_false()
    }

    fn unit_collection(&self, variables: &BddVariableSet) -> Bdd {
        // The only values that are correct in this encoding are values where `x_{k}` implies
        // `x_{k-1}` for all valid `k`. Following such condition, once a symbolic variable is
        // `true`, all "smaller" variables must be also `true`.

        // TODO:
        //  We might cache this value in the `SymbolicDomain` object so it does not need
        //  to be recomputed every time and we can just copy it instead.

        let mut true_set = variables.mk_true();
        for k in 1..self.variables.len() {
            let var_k = self.variables[k];
            let var_k_minus_one = self.variables[k - 1];
            let var_k = variables.mk_var(var_k);
            let var_k_minus_one = variables.mk_var(var_k_minus_one);
            let k_implies_k_minus_one = var_k.imp(&var_k_minus_one);
            true_set = true_set.and(&k_implies_k_minus_one);
        }
        true_set
    }
}

/// Just a type alias for a boxed generic symbolic domain object that can be
/// used to encode `u8` types.
pub type GenericIntegerDomain = Box<dyn SymbolicDomain<u8>>;

/// `GenericSymbolicStateSpace` is a collection of `SymbolicDomain` objects that together encode
/// the state space of a logical model.
///
/// This implementation allows using different encodings for individual variables, which may lead
/// to minor inefficiencies due to dynamic dispatch. In the future, once we select an optimal
/// encoding technique, we can also provide fully specialized structures relying on such a single
/// encoding instead.
///
/// TODO:
///     - Here, we will be using just plain `usize` integers as identifiers for model variables.
///     However, in the future we could introduce a `VariableId` type or something similar to make
///     this a bit more safe/explicit.
///     - Here, we are just using `Vec<u8>` as a representation of a network state. In the future,
///     it would be nice to have a proper `State` type (tied to the `VariableId` type).
///     - Many of the methods now need linear time to finish computing (i.e. iterate through all
///     variable domains). In some cases (like `Self::symbolic_variables`), we should just cache
///     the result in the constructor and then copy it when the method is called.
#[derive(Clone)]
pub struct GenericStateSpaceDomain {
    variable_domains: Vec<GenericIntegerDomain>,
}

impl GenericStateSpaceDomain {
    /// This creates a new `GenericStateSpaceDomain` from a list of `GenericIntegerDomain` objects.
    pub fn new(variable_domains: Vec<GenericIntegerDomain>) -> GenericStateSpaceDomain {
        GenericStateSpaceDomain { variable_domains }
    }

    /// Get the reference to one of the inner variable domains. For example if we want to create
    /// a set that "restricts" just one of the variables.
    ///
    /// WARNING: By creating encodings using the "raw" domains of individual variables, you are
    /// effectively ignoring the `empty_collection`/`unit_collection` constraints of the remaining
    /// domains in this state space. Specifically, if a domain of some other variable (i.e. not
    /// `variable_id`) admits some invalid encoded values (i.e. `unit_collection` is not a `true`
    /// BDD), then BDDs created using such domains will end up with these invalid values as well.
    /// You can fix this by explicitly intersecting the result with `Self::unit_collection` to
    /// remove the invalid values.
    pub fn get_raw_domain(&self, variable_id: usize) -> &GenericIntegerDomain {
        &self.variable_domains[variable_id]
    }

    /*
       TODO:
           - We should add some more "user friendly" API that will hide the issue described
           in the warning above (i.e. automatically ensure that only valid encodings can be
           created. For example, something like `Self::encode_variable_collection`.
           - Other option would be to create some `SymbolicSet` type which would actually keep
           track of the relevant symbolic domain. That way, we would know that BDD objects created
           using a `GenericIntegerDomain` are not compatible with BDD objects created using a
           `GenericStateSpaceDomain` and we could add the explicit sanitization step into the
           API of the `SymbolicSet` type.
    */
}

impl SymbolicDomain<Vec<u8>> for GenericStateSpaceDomain {
    fn encode_bits(&self, bdd_valuation: &mut BddPartialValuation, value: &Vec<u8>) {
        for (i, domain) in self.variable_domains.iter().enumerate() {
            domain.encode_bits(bdd_valuation, &value[i])
        }
    }

    fn decode_bits(&self, bdd_valuation: &BddPartialValuation) -> Vec<u8> {
        self.variable_domains
            .iter()
            .map(|domain| domain.decode_bits(bdd_valuation))
            .collect()
    }

    fn symbolic_variables(&self) -> Vec<BddVariable> {
        let mut result = Vec::new();
        for domain in &self.variable_domains {
            result.extend(domain.symbolic_variables());
        }
        result
    }

    fn symbolic_size(&self) -> usize {
        let mut result = 0;
        for domain in &self.variable_domains {
            result += domain.symbolic_size();
        }
        result
    }

    fn empty_collection(&self, variables: &BddVariableSet) -> Bdd {
        let mut result = variables.mk_false();
        for domain in &self.variable_domains {
            result = result.or(&domain.empty_collection(variables));
        }
        result
    }

    fn unit_collection(&self, variables: &BddVariableSet) -> Bdd {
        let mut result = variables.mk_true();
        for domain in &self.variable_domains {
            result = result.and(&domain.unit_collection(variables));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::symbolic_domain::{GenericStateSpaceDomain, SymbolicDomain, UnaryIntegerDomain};
    use biodivine_lib_bdd::BddVariableSetBuilder;

    // TODO:
    //      - These tests are quite "fat", maybe several smaller unit tests would be in order.
    //      - These tests do not cover any failures. Several tests for different panic/contract
    //      violation scenarios are needed.

    #[test]
    pub fn test_unary_domain() {
        let mut builder = BddVariableSetBuilder::new();
        let domain = UnaryIntegerDomain::new(&mut builder, "x", 5);
        let var_set = builder.build();

        assert_eq!(domain.symbolic_size(), 5);
        assert_eq!(domain.symbolic_variables().len(), domain.symbolic_size());

        let unit_set = domain.unit_collection(&var_set);
        let decoded_unit_set = domain.decode_collection(&var_set, &unit_set);
        assert_eq!(decoded_unit_set.len(), 6);

        let empty_set = domain.empty_collection(&var_set);
        let decoded_empty_set = domain.decode_collection(&var_set, &empty_set);
        assert_eq!(decoded_empty_set.len(), 0);

        let test_set = vec![1, 2, 5];
        let encoded_test_set = domain.encode_collection(&var_set, &test_set);
        let decoded_test_set = domain.decode_collection(&var_set, &encoded_test_set);
        // In this particular encoding, the order of elements is preserved.
        assert_eq!(test_set, decoded_test_set);
    }

    #[test]
    pub fn test_generic_state_space() {
        let mut builder = BddVariableSetBuilder::new();
        let domain_x = Box::new(UnaryIntegerDomain::new(&mut builder, "x", 5));
        let domain_y = Box::new(UnaryIntegerDomain::new(&mut builder, "y", 14));
        let var_set = builder.build();
        let state_space = GenericStateSpaceDomain::new(vec![domain_x, domain_y]);

        assert_eq!(state_space.symbolic_size(), 5 + 14);
        assert_eq!(state_space.symbolic_variables(), var_set.variables());

        let empty_set = state_space.empty_collection(&var_set);
        let decoded_empty_set = state_space.decode_collection(&var_set, &empty_set);
        assert_eq!(decoded_empty_set.len(), 0);

        let unit_set = state_space.unit_collection(&var_set);
        let decoded_unit_set = state_space.decode_collection(&var_set, &unit_set);
        assert_eq!(decoded_unit_set.len(), 6 * 15);

        // Build a test set that is restricted to specific states.
        let test_set = vec![vec![0, 12], vec![1, 3], vec![5, 5]];
        let encoded_test_set = state_space.encode_collection(&var_set, &test_set);
        let decoded_test_set = state_space.decode_collection(&var_set, &encoded_test_set);
        assert_eq!(test_set, decoded_test_set);

        // Build a test set that is restricted in a specific variable (i.e. only x/y is restricted
        // and the remaining values are unconstrained).
        let restrict_x = vec![0, 2, 5];
        let restrict_y = vec![8, 11, 12, 13];

        let encoded_restrict_x = state_space
            .get_raw_domain(0)
            .encode_collection(&var_set, &restrict_x);
        let encoded_restrict_y = state_space
            .get_raw_domain(1)
            .encode_collection(&var_set, &restrict_y);
        // "Sanitize" the encoding by removing values which are invalid for the remaining variables.
        // This is necessary because the BDDs we just created do not depend on the second variable
        // at all, and hence if we "naively" extend them to the full state space domain, the invalid
        // encodings will still be present.
        let encoded_restrict_x = encoded_restrict_x.and(&unit_set);
        let encoded_restrict_y = encoded_restrict_y.and(&unit_set);

        let decoded_restrict_x = state_space.decode_collection(&var_set, &encoded_restrict_x);
        let decoded_restrict_y = state_space.decode_collection(&var_set, &encoded_restrict_y);

        // Here the x/y component is restricted to the values in `restrict_x/y` and the second
        // variable can be any value from its domain.
        assert_eq!(decoded_restrict_x.len(), 3 * 15);
        assert_eq!(decoded_restrict_y.len(), 6 * 4);

        let restrict_both = encoded_restrict_x.and(&encoded_restrict_y);

        let decoded_both = state_space.decode_collection(&var_set, &restrict_both);
        // Here, both variables are restricted to the values from `restrict_x/y`, but any
        // combination of such values is allowed.
        assert_eq!(decoded_both.len(), 3 * 4);
    }
}