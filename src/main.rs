use rayon::prelude::*;
use std::collections::HashMap;
use std::io::stdin;
// use std::process::Output;

const ONE: i32 = 1;

struct Structure {
    structure: HashMap<i32, Vec<i32>>,
}

// struct Error {
//     veracity: bool,
//     err: String,
// }

impl Structure {
    fn get_slice(&self, generation: &i32, center: &i32) -> Result<&Vec<i32>, String> {
        let slice = match self.structure.get(generation) {
            Some(value) => Ok(value),
            None => Err(format!(
                "could not get slice at {} for generation {}",
                *center, *generation
            )),
        };

        return slice;
    }
}

struct Rule {
    rule: HashMap<Vec<i32>, i32>,
}

struct InitialCondition {
    size: usize,
    state: Vec<i32>,
}

struct CellularAutomaton {
    rule: Rule,
    size: i32,
    final_generation: i32,
    structure: Structure,
}

impl CellularAutomaton {
    fn run_cellular_automaton(
        &mut self,
        rule: Rule,
        mut initial_condition: InitialCondition,
        final_generation: i32,
    ) {
        let number_of_threads = initial_condition.size;
        let mut structure = Structure {
            structure: HashMap::new(),
        };
        initial_condition.state.insert(0, ONE);
        initial_condition.state.push(ONE);
        structure.structure.insert(0, initial_condition.state);

        // for generation in 0..final_generation {
        //     for thread in 0..number_of_threads {
        //         let slice: &[i32];

        //         match structure.structure.get(&generation) {
        //             Some(generation_state) => slice = &generation_state[thread - 1..thread + 1],
        //             None => println!(
        //                 "could not find generation number in automaton map{}",
        //                 generation - 1
        //             ),
        //         }

        //         let rule_output = match rule.rule.get(slice) {
        //             Some(value) => value,
        //             None => &9,
        //         };

        //         match structure.structure.get_mut(&(generation + 1)) {
        //             Some(generation_state) => {
        //                 if generation_state.len() < thread {
        //                     generation_state.push(*rule_output);
        //                 } else {
        //                     generation_state.insert(thread, *rule_output)
        //                 }
        //             }
        //             None => {
        //                 structure
        //                     .structure
        //                     .insert(generation + 1, vec![*rule_output]);
        //             }
        //         }
        //     }
        // }

        for generation in 0..final_generation {
            structure.structure.insert(
                generation + 1,
                (0..number_of_threads as i32)
                    .into_par_iter()
                    .map(|i| {
                        let slice: &Vec<i32> = match structure.get_slice(&generation, &i) {
                            Ok(slice) => slice,
                            Err(_) => &vec![1, 1, 1],
                        };

                        match rule.rule.get(slice) {
                            Some(value) => *value,
                            None => 9,
                        }
                    })
                    .collect(),
            );
        }

        self.rule = rule;
        self.size = number_of_threads as i32;
        self.final_generation = final_generation;
        self.structure = structure;
    }
}
fn main() {
    let mut initial_state_string = String::new();
    let mut initial_state_vec: Vec<i32> = Vec::new();
    let mut initial_condition: InitialCondition = InitialCondition {
        size: (0),
        state: (Vec::new()),
    };

    println!("Hello, world!");

    println!("Enter initial state");
    stdin()
        .read_line(&mut initial_state_string)
        .expect("fialed to read initial state");

    for state in initial_state_string.chars() {
        match state.to_digit(10) {
            Some(n) => {
                initial_state_vec.push(n as i32);
                println!("{}",n);
            }
            None => continue,
        };
    }

    initial_condition.size = initial_state_vec.len();
    initial_condition.state = initial_state_vec;
}
