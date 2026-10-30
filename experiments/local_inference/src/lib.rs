#![no_std]
#![forbid(unsafe_code)]

/// Synthetic integer observations, each restricted to 0..=100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input { pressure:i16, cpu_load:i16, io_activity:i16 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

impl Input {
    pub fn new(pressure:i16, cpu_load:i16, io_activity:i16) -> Result<Self, OutOfRange> {
        if [pressure,cpu_load,io_activity].iter().any(|v| !(0..=100).contains(v)) { return Err(OutOfRange); }
        Ok(Self { pressure, cpu_load, io_activity })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output { pub hidden:[i32;4], pub scores:[i32;2] }

// Hand-written synthetic coefficients: no training, external model or system policy.
const HIDDEN_WEIGHTS:[[i32;3];4]=[[3,-2,1],[-1,4,-1],[2,1,2],[-2,-1,3]];
const HIDDEN_BIAS:[i32;4]=[-50,20,-100,30];
const OUTPUT_WEIGHTS:[[i32;4];2]=[[2,-1,1,-1],[-1,1,2,1]];
const OUTPUT_BIAS:[i32;2]=[10,-20];

/// Dense integer layer, ReLU, then two linear integer scores.
/// For valid inputs, hidden maxima are [350,420,400,330]; all arithmetic fits i32.
pub fn infer(input:Input)->Output {
    let values=[input.pressure as i32,input.cpu_load as i32,input.io_activity as i32];
    let mut hidden=[0i32;4];
    for row in 0..4 {
        let mut sum=HIDDEN_BIAS[row];
        for (column,value) in values.iter().enumerate() {sum+=HIDDEN_WEIGHTS[row][column]*value;}
        hidden[row]=sum.max(0);
    }
    let mut scores=[0i32;2];
    for row in 0..2 {
        let mut sum=OUTPUT_BIAS[row];
        for (column,value) in hidden.iter().enumerate() {sum+=OUTPUT_WEIGHTS[row][column]*value;}
        scores[row]=sum;
    }
    Output {hidden,scores}
}
