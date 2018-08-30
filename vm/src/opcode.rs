//! Instruction in the VM;
//! Each Instruction in the VM is 32 bits and the first 8 bits will
//! contain our opcode
//! The remainaing 24 bits will be used for the Operands

pub type OpCode = u8;

/// ILLEGAL INST
pub const IGL: u8 = 0x0;

/// HLT
/// Stops the running of the vm
pub const HLT: u8 = 0x1;

/// JMP Dest
/// Changes the ip to the value in the register
/// Allows for jumping forward or backwards
pub const JMP: u8 = 0x2;

/// JMPF DEST
/// increments the `ip` by the value stored in DEST
pub const JMPF: u8 = 0x3;

/// JMPB DEST
/// decrements the `ip` by the value stored in DEST
pub const JMPB: u8 = 0x4;

/// JMPS if the equal flag is set;
pub const JMPEQ: u8 = 0x5;

/// JMPS if the equal flag is not set;
pub const JMPNEQ: u8 = 0x6;

/// ADD SRC SRC DEST
pub const ADD: u8 = 0x7;

/// SUB SRC SRC DEST
pub const SUB: u8 = 0x8;

/// MUL SRC SRC DEST
pub const MUL: u8 = 0x9;

/// DIV SRC SRC DEST
pub const DIV: u8 = 0x10;

/// NOT
/// Set the equal_flag to !equal_flag
pub const NOT: u8 = 0x11;

/// EQUAL SRC SRC
/// Sets the equal_flag to true
pub const EQUAL: u8 = 0x12;

/// GREATER SRC SRC
/// Sets the equal_flag to true
pub const GREATER: u8 = 0x13;

/// Less SRC SRC
/// Sets the equal_flag to 1
pub const LESS: u8 = 0x14;

/// LOAD SRC DEST
pub const LOAD: u8 = 0x15;

/// STORES $SRC $DEST
/// stores the value in src in dest
pub const STORE: u8 = 0x16;

/// ALLOC $BYTES
/// Extends the heap by n bytes
pub const ALLOC: u8 = 0x17;

/// FREE $BYTES
/// Shrinks the heap by n bytes
pub const FREE: u8 = 0x18;

/// INC $REG
/// Increase the value stored in the register by 1
pub const INC: u8 = 0x19;

/// DEC $REG
/// Decrease the value stored in the register by 1
pub const DEC: u8 = 0x20;

/// PUSH $REG
/// Pushes the value onto the stack
pub const PUSH: u8 = 0x21;

/// POP $REG
/// Popes the value off the top of stack
pub const POP: u8 = 0x22;