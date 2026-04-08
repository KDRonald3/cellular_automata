use rayon::prelude::*;
use std::collections::HashMap;
use std::io::stdin;
// use std::process::Output;

const ONE: i32 = 1;
const ZERO: i32 = 0;

struct Structure {
    /*
    Stores the generation of a cellular aotomaton, by associating the generation with the state of the automaton
     */
    structure: HashMap<i32, Vec<i32>>,
}

struct Rule {
    /*
    This stores the rule number in base 10 and the rule in binary representation.
    number represents the base ten rule,
    rule uses a hashmap to associate the state to be updated, and the result as defined by the rule
     */
    number: i32,
    rule: HashMap<Vec<i32>, i32>,
}

impl Rule {
    fn base_10_to_2(&self, number_base_10: &i32, length: usize) -> Vec<i32> {
        /*
        This converts any base 10 number given to it, to a vector of the binary representation of that number.
        VARIABLES:
        &self: the rule struct on whic the method is called,
        number_base_10: the base ten number to be converted,
        Length: the prefered minimun length of the base 2 representation
         */
        let mut number = *number_base_10;
        let mut number_base_2 = Vec::new();

        while number != ZERO {
            number_base_2.insert(ZERO as usize, number % 2);
            number = (number - number % 2) / 2;
        }
        while number_base_2.len() < length {
            number_base_2.insert(ZERO as usize, ZERO);
        }

        return number_base_2;
    }

    fn create_rule(&mut self) {
        /*
        create_rule associates cell update states to values defined the the rule
        VARIABLES:
        &self: This is the rule struct on which the method is called
         */
        let rule_number_base_2 = self.base_10_to_2(&self.number, 8);
        for i in ZERO..8 {
            self.rule
                .insert(self.base_10_to_2(&i, 3), rule_number_base_2[i as usize]);
        }
    }
}
struct InitialCondition {
    /*
    Struct holding initial condition entered by the user.
    size: defines the 1 dimensional length of the automaton
    final_generation: How many evolutions of the automaton should be run
    state: defines generation zero of the automaton
     */
    size: usize,
    final_generation: i32,
    state: Vec<i32>,
}

struct CellularAutomaton {
    /*
    This holds all the data about any cellular automaton
    rule: stores the rule to be run on the automaton
    initial_condition: hold the condition defined by the user for the automaton
    structure:hold the output of evolving the automaton for the desired number of generations
     */
    rule: Rule,
    initial_condition: InitialCondition,
    structure: Structure,
}

impl CellularAutomaton {
    fn run(&mut self) {
        /*
        CellularAutomata.run() evolves the cellular automaton
        &self contains all the data needed to do so.
         */
        let mut structure = Structure {
            structure: HashMap::new(),
        };
        self.initial_condition.state.insert(ZERO as usize, ONE);
        self.initial_condition.state.push(ONE);
        structure.structure.insert(ZERO, self.initial_condition.state.clone());

        for generation in ZERO..self.initial_condition.final_generation {
            structure.structure.insert(
                generation + 1,
                (ZERO..self.initial_condition.size as i32)
                    .into_par_iter()
                    .map(|i| {
                        let slice: &[i32] = match structure.structure.get(&generation) {
                            Some(vec_slice) => &vec_slice[i as usize..i as usize + 3],
                            None => &vec![1, 1, 1][0..3],
                        };

                        match self.rule.rule.get(slice) {
                            Some(value) => *value,
                            None => 9,
                        }
                    })
                    .collect(),
            );

            structure.structure.entry(generation + 1).and_modify(|f| {
                f.insert(ZERO as usize, ONE);
                f.push(ONE);
            });
        }

        self.structure = structure;
    }
}

fn main() {
    let mut initial_state_string = String::new();
    let mut initial_state_vec: Vec<i32> = Vec::new();
    let mut initial_condition: InitialCondition = InitialCondition {
        size: (ZERO as usize),
        final_generation: ZERO,
        state: (Vec::new()),
    };
    let mut rule_number_string: String = String::new();
    let mut final_generation_string: String = String::new();

    println!("Enter initial state");
    stdin()
        .read_line(&mut initial_state_string)
        .expect("fialed to read initial state");
    initial_state_string = initial_state_string.split("\r\n").collect();

    for state in initial_state_string.chars() {
        match state.to_digit(10) {
            Some(n) => {
                initial_state_vec.push(n as i32);
                println!("{}", n);
            }
            None => continue,
        };
    }
    println!("Enter final generation: ");
    stdin()
        .read_line(&mut final_generation_string)
        .expect("failed to read generation");
    let final_generation: i32 = final_generation_string
        .split("\r\n")
        .collect::<String>()
        .parse::<i32>()
        .unwrap();

    initial_condition.size = initial_state_vec.len();
    initial_condition.final_generation = final_generation;
    initial_condition.state = initial_state_vec;

    println!("Enter rule number: ");
    stdin()
        .read_line(&mut rule_number_string)
        .expect("could not read rule number");

    let rule_number = rule_number_string
        .split("\r\n")
        .collect::<String>()
        .parse::<i32>()
        .unwrap();

    let mut rule = Rule {
        number: rule_number,
        rule: HashMap::new(),
    };
    rule.create_rule();


    let mut cellular_automaton: CellularAutomaton = CellularAutomaton {
        rule,
        initial_condition,
        structure: Structure {
            structure: HashMap::new(),
        },
    };

    cellular_automaton.run();
    for i in 0..cellular_automaton.initial_condition.final_generation {
        println!("{:?}", cellular_automaton.structure.structure.get(&i))
    }
}
