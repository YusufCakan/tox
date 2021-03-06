use super::infer::types::{Type, TypeCon};
use crate::ast;
use fnv::FnvHashMap;
use opcode;
use std::hash::Hash;
use util::emmiter::Reporter;
use util::pos::{Span, Spanned};
use util::symbol::{Symbol, Symbols};
use vm::{Chunk, Class, Function, FunctionObject, Program, RawObject, StringObject, Value};
type ParseResult<T> = Result<T, ()>;

#[derive(Debug, Clone, Copy)]
struct LoopDescription {
    /// The index of the start label
    start: usize,
    /// The index of the end label
    end: usize,
}

#[derive(Debug, Clone)]
pub struct StackedMap<K: Hash + Eq, V: Clone> {
    table: FnvHashMap<K, Vec<V>>,
    scopes: Vec<Option<K>>,
}

impl<K: Hash + Eq + Copy, V: Clone> StackedMap<K, V> {
    pub fn new() -> Self {
        StackedMap {
            table: FnvHashMap::default(),
            scopes: vec![],
        }
    }

    pub fn begin_scope(&mut self) {
        self.scopes.push(None);
    }

    pub fn end_scope(&mut self) {
        while let Some(Some(value)) = self.scopes.pop() {
            let mapping = self.table.get_mut(&value).expect("Symbol not in Symbols");
            mapping.pop();
        }
    }

    /// Enters a peice of data into the current scope
    pub fn insert(&mut self, key: K, value: V) {
        let mapping = self.table.entry(key).or_insert_with(Vec::new);
        mapping.push(value);

        self.scopes.push(Some(key));
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.table.get(key).and_then(|vec| vec.last())
    }
}
pub struct Builder<'a> {
    /// The current chunk
    chunk: Chunk,
    /// A count of all local vars
    /// The number is the postion of the local on the local stack
    locals: StackedMap<Symbol, usize>,

    params: FnvHashMap<Symbol, usize>,
    current_loop: Option<LoopDescription>,
    ///  A linked list of all the objects allocated. This
    /// is passed to the vm so runtime collection can be done
    pub objects: RawObject,

    symbols: &'a Symbols<()>,
    /// The reporter used to reporter any errors
    reporter: &'a mut Reporter,
    /// The slot of the variable
    slots: u32,
    ///
    line: u32,
}

impl<'a> Builder<'a> {
    pub fn new(
        reporter: &'a mut Reporter,
        symbols: &'a Symbols<()>,
        objects: RawObject,
        params: FnvHashMap<Symbol, usize>,
    ) -> Self {
        Builder {
            chunk: Chunk::new(),
            locals: StackedMap::new(),
            line: 0,
            slots: 0,
            current_loop: None,
            symbols,
            params,
            objects,
            reporter,
        }
    }

    pub fn emit_byte(&mut self, byte: u8) {
        self.chunk.write(byte, self.line)
    }

    pub fn new_slot(&mut self) -> u32 {
        let slot = self.slots;
        self.slots += 1;
        slot
    }

    pub fn patch_jump(&mut self, offset: usize) {
        // -2 to adjust for the bytecode for the jump offset itself.
        let jump = self.chunk.code.len() - offset - 2;

        self.chunk.code[offset] = ((jump >> 8) & 0xff) as u8;
        self.chunk.code[offset + 1] = (jump & 0xff) as u8;
    }

    pub fn emit_jump(&mut self, byte: u8) -> usize {
        self.emit_byte(byte);
        self.emit_bytes(0xff, 0xff);
        self.chunk.code.len() - 2
    }

    pub fn emit_loop(&mut self, loop_start: usize) {
        self.emit_byte(opcode::LOOP);

        let offset = self.chunk.code.len() - loop_start + 2;

        self.emit_bytes(((offset >> 8) & 0xff) as u8, (offset & 0xff) as u8)
    }

    pub fn emit_bytes(&mut self, byte1: u8, byte2: u8) {
        self.emit_byte(byte1);
        self.emit_byte(byte2);
    }

    pub fn emit_constant(&mut self, constant: Value, span: Span) -> ParseResult<()> {
        let value = self.make_constant(constant, span)?;
        self.emit_bytes(opcode::CONSTANT, value);
        Ok(())
    }

    pub fn make_constant(&mut self, value: Value, span: Span) -> ParseResult<u8> {
        let index = self.chunk.add_constant(value);

        if index > 256 {
            self.reporter.error("too many constants in one chunk", span);
            Err(())
        } else {
            Ok(index as u8)
        }
    }

    pub fn set_span(&mut self, span: Span) {
        if span.start.line > self.line {
            self.line = span.start.line
        }
    }

    pub fn compile_statement(
        &mut self,
        statement: &Spanned<ast::TypedStatement>,
    ) -> ParseResult<()> {
        use crate::ast::Statement;
        self.set_span(statement.span);
        match statement.value.statement.value {
            Statement::Block(ref statements) => {
                self.locals.begin_scope();

                for statement in statements {
                    self.compile_statement(statement)?;
                }
                self.locals.end_scope();

                Ok(())
            }

            Statement::Break => {
                let description = self.current_loop.expect("Using break outside a loop");

                self.emit_bytes(opcode::JUMP, description.end as u8);

                Ok(())
            }

            Statement::Continue => {
                let description = self.current_loop.expect("Using break outside a loop");

                self.emit_bytes(opcode::JUMP, description.start as u8);
                Ok(())
            }

            Statement::Expr(ref expr) => {
                self.compile_expression(expr)?;

                Ok(())
            }

            Statement::Print(ref expr) => {
                self.compile_expression(expr)?;

                self.emit_byte(opcode::PRINT);
                Ok(())
            }

            Statement::Return(ref expr) => {
                self.compile_expression(expr)?;

                self.emit_byte(opcode::RETURN);

                Ok(())
            }

            Statement::If {
                ref cond,
                ref then,
                otherwise: None,
            } => {
                self.compile_expression(cond)?;

                let false_label = self.emit_jump(opcode::JUMPNOT);

                self.emit_byte(opcode::POP);

                self.compile_statement(then)?;

                self.patch_jump(false_label);

                self.emit_byte(opcode::POP);

                Ok(())
            }

            Statement::If {
                ref cond,
                ref then,
                otherwise: Some(ref otherwise),
            } => {
                self.compile_expression(cond)?;

                let false_label = self.emit_jump(opcode::JUMPNOT);

                self.emit_byte(opcode::POP);

                self.compile_statement(then)?;

                let end_label = self.emit_jump(opcode::JUMP);

                self.patch_jump(false_label);

                self.emit_byte(opcode::POP);

                self.compile_statement(otherwise)?;

                self.patch_jump(end_label);

                Ok(())
            }

            Statement::Let {
                ref ident,
                ref expr,
                ..
            } => {
                //
                if let Some(ref expr) = *expr {
                    self.compile_expression(expr)?;
                } else {
                    self.emit_constant(Value::nil(), statement.span)?;
                } // Compile the expression

                let slot = self.new_slot();

                self.locals.insert(*ident, slot as usize);

                self.emit_bytes(opcode::SETLOCAL, slot as u8); // Write the symbol id

                Ok(())
            }

            Statement::While(ref cond, ref body) => {
                let start_label = self.chunk.code.len();

                self.compile_expression(cond)?;

                let out = self.emit_jump(opcode::JUMPNOT);

                self.current_loop = Some(LoopDescription {
                    start: start_label,
                    end: out,
                });

                self.emit_byte(opcode::POP);

                self.compile_statement(body)?;

                self.emit_loop(start_label); // Jumps back to the start

                self.patch_jump(out); // the outer label

                self.emit_byte(opcode::POP); //removes cond from stack

                Ok(())
            }
        }
    }

    pub fn compile_expression(&mut self, expr: &Spanned<ast::TypedExpression>) -> ParseResult<()> {
        use crate::ast::{AssignOperator, Expression, Literal, Op};
        self.set_span(expr.span);

        match expr.value.expr.value {
            Expression::Assign(ref ident, ref op, ref expr) => {
                let pos = if let Some(pos) = self.locals.get(ident) {
                    *pos
                } else if let Some(pos) = self.params.get(ident) {
                    *pos
                } else {
                    unreachable!(); // Params are treated as locals so it should be present
                };

                match *op {
                    AssignOperator::Equal => {
                        self.compile_expression(expr)?;
                        self.emit_bytes(opcode::SETLOCAL, pos as u8);
                    }
                    AssignOperator::MinusEqual => {
                        self.emit_bytes(opcode::GETLOCAL, pos as u8); // get the var

                        let opcode = match expr.value.ty {
                            Type::App(TypeCon::Int, _) => opcode::SUB,
                            Type::App(TypeCon::Float, _) => opcode::SUBF,
                            _ => unreachable!(), // type checker should prevent this
                        };

                        self.compile_expression(expr)?; // get the expr

                        self.emit_byte(opcode);

                        self.emit_bytes(opcode::SETLOCAL, pos as u8); // store it in x
                    }

                    AssignOperator::PlusEqual => {
                        self.emit_bytes(opcode::GETLOCAL, pos as u8); // get the var

                        let opcode = match expr.value.ty {
                            Type::App(TypeCon::Int, _) => opcode::ADD,
                            Type::App(TypeCon::Float, _) => opcode::ADDF,
                            _ => unreachable!(), // type checker should prevent this
                        };

                        self.compile_expression(expr)?; // get the expr

                        self.emit_byte(opcode);

                        self.emit_bytes(opcode::SETLOCAL, pos as u8); // store it in x
                    }

                    AssignOperator::SlashEqual => {
                        self.emit_bytes(opcode::GETLOCAL, pos as u8); // get the var

                        let opcode = match expr.value.ty {
                            Type::App(TypeCon::Int, _) => opcode::DIV,
                            Type::App(TypeCon::Float, _) => opcode::DIVF,
                            _ => unreachable!(), // type checker should prevent this
                        };

                        self.compile_expression(expr)?; // get the expr

                        self.emit_byte(opcode);

                        self.emit_bytes(opcode::SETLOCAL, pos as u8); // store it in x
                    }

                    AssignOperator::StarEqual => {
                        self.emit_bytes(opcode::GETLOCAL, pos as u8); // get the var

                        let opcode = match expr.value.ty {
                            Type::App(TypeCon::Int, _) => opcode::MUL,
                            Type::App(TypeCon::Float, _) => opcode::MULF,
                            _ => unreachable!(), // type checker should prevent this
                        };

                        self.compile_expression(expr)?; // get the expr

                        self.emit_byte(opcode);

                        self.emit_bytes(opcode::SETLOCAL, pos as u8); // store it in x
                    }
                }
            }

            Expression::Array(ref exprs) => {
                for expr in exprs.iter().rev() {
                    // reverse because items how items are popped off the stack
                    self.compile_expression(expr)?;
                }

                self.emit_bytes(opcode::ARRAY, exprs.len() as u8);
            }

            Expression::Index(ref target, ref index) => {
                match expr.value.ty {
                    Type::App(TypeCon::Str, _) => {
                        self.compile_expression(target)?;
                        self.compile_expression(index)?;

                        self.emit_byte(opcode::INDEXSTRING);
                    }

                    Type::App(TypeCon::Array(_), _) => {
                        self.compile_expression(target)?;
                        self.compile_expression(index)?;

                        self.emit_byte(opcode::INDEXARRAY);
                    }

                    _ => unreachable!(), // Type checking should prevent this being reached
                }
            }

            Expression::Literal(ref literal) => match *literal {
                Literal::False(_) => {
                    self.emit_byte(opcode::FALSE);
                }
                Literal::True(_) => {
                    self.emit_byte(opcode::TRUE);
                }
                Literal::Nil => {
                    self.emit_byte(opcode::NIL);
                }
                Literal::Int(ref n) => {
                    self.emit_constant(Value::int(*n), expr.value.expr.span)?;
                }
                Literal::Float(ref f) => {
                    self.emit_constant(Value::float(*f), expr.value.expr.span)?;
                }
                Literal::Str(ref string) => {
                    let object = StringObject::new(string, self.objects);

                    self.emit_constant(Value::object(object), expr.value.expr.span)?;
                }
            },

            Expression::Binary(ref lhs, ref op, ref rhs) => {
                if *op == Op::And {
                    self.compile_and(lhs, rhs)?;
                } else if *op == Op::Or {
                    self.compile_or(lhs, rhs)?;
                } else {
                    self.compile_expression(lhs)?;
                    self.compile_expression(rhs)?;

                    match (&expr.value.ty, op) {
                        (Type::App(TypeCon::Int, _), Op::Plus) => self.emit_byte(opcode::ADD),
                        (Type::App(TypeCon::Float, _), Op::Plus) => self.emit_byte(opcode::ADDF),

                        (Type::App(TypeCon::Int, _), Op::Minus) => self.emit_byte(opcode::SUB),
                        (Type::App(TypeCon::Float, _), Op::Minus) => self.emit_byte(opcode::SUBF),

                        (Type::App(TypeCon::Int, _), Op::Slash) => self.emit_byte(opcode::DIV),
                        (Type::App(TypeCon::Float, _), Op::Slash) => self.emit_byte(opcode::DIVF),

                        (Type::App(TypeCon::Int, _), Op::Star) => self.emit_byte(opcode::MUL),
                        (Type::App(TypeCon::Float, _), Op::Star) => self.emit_byte(opcode::MULF),

                        // For comparisson the lhs and the rhs should be the same so only
                        // check the type of the lhs
                        (Type::App(TypeCon::Bool, _), Op::LessThan) => match lhs.value.ty {
                            Type::App(TypeCon::Int, _) => self.emit_byte(opcode::LESS),
                            Type::App(TypeCon::Float, _) => self.emit_byte(opcode::LESSF),
                            _ => unreachable!(),
                        },

                        (Type::App(TypeCon::Bool, _), Op::LessThanEqual) => match lhs.value.ty {
                            Type::App(TypeCon::Int, _) => {
                                self.emit_bytes(opcode::LESS, opcode::NOT)
                            }
                            Type::App(TypeCon::Float, _) => {
                                self.emit_bytes(opcode::LESSF, opcode::NOT)
                            }
                            _ => unreachable!(),
                        },

                        (Type::App(TypeCon::Bool, _), Op::GreaterThan) => match lhs.value.ty {
                            Type::App(TypeCon::Int, _) => self.emit_byte(opcode::GREATER),
                            Type::App(TypeCon::Float, _) => self.emit_byte(opcode::GREATERF),
                            _ => unreachable!(),
                        },

                        (Type::App(TypeCon::Bool, _), Op::GreaterThanEqual) => match lhs.value.ty {
                            Type::App(TypeCon::Int, _) => {
                                self.emit_bytes(opcode::GREATER, opcode::NOT)
                            }
                            Type::App(TypeCon::Float, _) => {
                                self.emit_bytes(opcode::GREATERF, opcode::NOT)
                            }
                            _ => unreachable!(),
                        },

                        (Type::App(TypeCon::Str, _), Op::Plus) => self.emit_byte(opcode::CONCAT),

                        (_, Op::EqualEqual) => self.emit_byte(opcode::EQUAL),
                        (_, Op::BangEqual) => self.emit_bytes(opcode::EQUAL, opcode::NOT),

                        // #[cfg(not(feature = "debug"))]
                        // _ => unsafe {
                        //     ::std::hint::unreachable_unchecked() // only in release mode for that extra speed boost
                        // },

                        // #[cfg(feature = "debug")]
                        (ref ty, ref op) => unimplemented!(" ty {:?} op {:?}", ty, op),
                    }
                }
            }

            Expression::Cast(ref from, ref to) => {
                self.compile_expression(from)?;

                match (&from.value.ty, &to) {
                    (Type::App(TypeCon::Int, _), Type::App(TypeCon::Float, _)) => {
                        self.emit_byte(opcode::INT2FLOAT)
                    }

                    (Type::App(TypeCon::Float, _), Type::App(TypeCon::Int, _)) => {
                        self.emit_byte(opcode::FLOAT2INT)
                    }

                    (Type::App(TypeCon::Bool, _), Type::App(TypeCon::Int, _)) => {
                        self.emit_byte(opcode::BOOL2INT)
                    }

                    (Type::App(TypeCon::Int, _), Type::App(TypeCon::Str, _)) => {
                        self.emit_byte(opcode::INT2STR)
                    }

                    (Type::App(TypeCon::Float, _), Type::App(TypeCon::Str, _)) => {
                        self.emit_byte(opcode::FLOAT2STR)
                    }

                    _ => unreachable!(), // cast only allows int -> float, float -> int, bool -> int
                }
            }

            Expression::Call(ref callee, ref args) => {
                for arg in args {
                    self.compile_expression(arg)?;
                }

                let name = self.symbols.name(*callee);

                match name.as_str() {
                    "clock" | "random" | "read" | "fopen" => {
                        self.emit_bytes(opcode::CALLNATIVE, callee.0 as u8)
                    }
                    _ => {
                        self.emit_bytes(opcode::CALL, callee.0 as u8);
                        self.emit_byte(args.len() as u8)
                    }
                }
            }

            Expression::ClassLiteral {
                ref symbol,
                ref properties,
            } => {
                for property in properties.iter().rev() {
                    //rev because poped of stack
                    self.compile_expression(&property.value.expr)?;
                }

                self.emit_bytes(opcode::CLASSINSTANCE, symbol.0 as u8);
                self.emit_byte(properties.len() as u8);

                for property in properties.iter().rev() {
                    //rev because poped of stack
                    self.emit_byte(property.value.name.0 as u8);
                }
            }

            Expression::InstanceMethodCall {
                ref method_name,
                ref instance,
                ref params,
            } => {
                for param in params {
                    self.compile_expression(param)?;
                }

                self.compile_expression(instance)?;

                self.emit_byte(opcode::CALLINSTANCEMETHOD);
                self.emit_bytes(method_name.0 as u8, params.len() as u8);
            }

            Expression::StaticMethodCall {
                ref class_name,
                ref method_name,
                ref params,
            } => {
                for param in params {
                    self.compile_expression(param)?;
                }

                self.emit_byte(opcode::CALLSTATICMETHOD);
                self.emit_bytes(class_name.0 as u8, method_name.0 as u8);
                self.emit_byte(params.len() as u8);
            }

            Expression::GetProperty {
                ref property_name,
                ref property,
                ..
            } => {
                self.compile_expression(property)?;
                self.emit_bytes(opcode::GETPROPERTY, property_name.0 as u8)
            }

            Expression::GetMethod {
                ref method_name,
                ref method,
                ..
            } => {
                self.compile_expression(method)?;
                self.emit_bytes(opcode::GETMETHOD, method_name.0 as u8)
            }

            Expression::Grouping(ref expr) => {
                self.compile_expression(expr)?;
            }

            Expression::Match { ref cond, ref arms } => {
                self.compile_expression(cond)?;

                let mut jumps = Vec::new();

                for arm in arms.value.iter() {
                    println!("{}", arm.value.is_all);
                    if arm.value.is_all {
                        self.compile_statement(&arm.value.body)?;
                        jumps.push(self.emit_jump(opcode::JUMP));
                    } else {
                        self.compile_expression(arm.value.pattern.as_ref().unwrap())?;
                        self.compile_expression(cond)?; //TODO: check if legal i.e if a+1
                        self.emit_byte(opcode::EQUAL);

                        let offset = self.emit_jump(opcode::JUMPNOT);

                        self.compile_statement(&arm.value.body)?;
                        jumps.push(self.emit_jump(opcode::JUMP));

                        self.patch_jump(offset);
                    }
                }

                for label in jumps {
                    self.patch_jump(label);
                }
            }

            Expression::Ternary(ref cond, ref if_true, ref if_false) => {
                self.compile_expression(cond)?;

                let false_label = self.emit_jump(opcode::JUMPNOT);

                self.compile_expression(if_true)?;

                let end_label = self.emit_jump(opcode::JUMP);

                self.patch_jump(false_label);

                self.compile_expression(if_false)?;

                self.patch_jump(end_label);
            }

            Expression::Unary(ref op, ref expr) => {
                use crate::ast::UnaryOp;

                self.compile_expression(expr)?;

                match *op {
                    UnaryOp::Bang => {
                        self.emit_byte(opcode::NOT);
                    }

                    UnaryOp::Minus => match &expr.value.ty {
                        Type::App(TypeCon::Int, _) => self.emit_byte(opcode::NEGATE),
                        Type::App(TypeCon::Float, _) => self.emit_byte(opcode::NEGATEF),
                        _ => unreachable!(),
                    },
                }
            }

            Expression::Var(ref ident, _) => {
                if let Some(pos) = self.locals.get(ident).cloned() {
                    self.emit_bytes(opcode::GETLOCAL, pos as u8);
                } else if let Some(offset) = self.params.get(ident).cloned() {
                    self.emit_bytes(opcode::GETPARAM, offset as u8);
                } else {
                    self.reporter.error("Undefined variable", expr.span);
                    return Err(()); // Params are treated as locals so it should be present
                }
            }

            Expression::VariantNoData {
                ref enum_name,
                ref tag,
            } => {
                self.emit_byte(opcode::ENUM);
                self.emit_bytes(enum_name.value.0 as u8, *tag as u8);
            }

            Expression::VariantWithData {
                ref enum_name,
                ref tag,
                ref inner,
            } => {
                self.emit_byte(opcode::ENUM);
                self.emit_bytes(enum_name.value.0 as u8, *tag as u8);
                self.compile_expression(inner)?;
            }

            Expression::Closure(ref func) => {
                let closure = compile_function(func, self.symbols, self.reporter, self.objects)?;

                let func = FunctionObject::new(closure.params.len(), closure, self.objects);

                self.emit_constant(Value::object(func), expr.span)?;
            }

            Expression::Set(ref property, ref instance, ref value) => {
                self.compile_expression(value)?;
                self.compile_expression(instance)?;
                self.emit_bytes(opcode::SETPROPERTY, property.0 as u8);
            }
        }

        Ok(())
    }

    fn compile_and(
        &mut self,
        lhs: &Spanned<ast::TypedExpression>,
        rhs: &Spanned<ast::TypedExpression>,
    ) -> ParseResult<()> {
        self.compile_expression(lhs)?;

        let false_label = self.emit_jump(opcode::JUMPNOT);

        self.compile_expression(rhs)?;

        self.patch_jump(false_label);

        Ok(())
    }

    fn compile_or(
        &mut self,
        lhs: &Spanned<ast::TypedExpression>,
        rhs: &Spanned<ast::TypedExpression>,
    ) -> ParseResult<()> {
        self.compile_expression(lhs)?;

        let else_label = self.emit_jump(opcode::JUMPIF);

        self.compile_expression(rhs)?;

        self.patch_jump(else_label);

        self.emit_byte(opcode::POP);

        Ok(())
    }
}

fn compile_class(
    class: &ast::Class,
    symbols: &Symbols<()>,
    reporter: &mut Reporter,
    objects: RawObject,
) -> ParseResult<Class> {
    let mut methods = FnvHashMap::default();

    for method in class.methods.iter() {
        methods.insert(
            method.name,
            compile_function(method, symbols, reporter, objects)?,
        );
    }

    Ok(Class {
        name: class.name,
        methods,
    })
}

fn compile_function(
    func: &ast::Function,
    symbols: &Symbols<()>,
    reporter: &mut Reporter,
    objects: RawObject,
) -> ParseResult<Function> {
    let mut params = FnvHashMap::default();

    for (i, param) in func.params.iter().enumerate() {
        params.insert(param.name, i);
    } // store param id and the index in the vec

    let mut builder = Builder::new(reporter, symbols, objects, params);

    builder.compile_statement(&func.body)?;

    Ok(Function {
        name: func.name,
        // locals: builder.locals,
        body: builder.chunk,
        params: builder.params,
    })
}

pub fn compile(
    ast: &ast::Program,
    symbols: &Symbols<()>,
    reporter: &mut Reporter,
) -> ParseResult<(Program, RawObject)> {
    let mut funcs = FnvHashMap::default();
    let mut classes: FnvHashMap<Symbol, Class> = FnvHashMap::default();

    let objects = ::std::ptr::null::<RawObject>() as RawObject;

    for function in ast.functions.iter() {
        funcs.insert(
            function.name,
            compile_function(function, symbols, reporter, objects)?,
        );
    }

    for class in ast.classes.iter() {
        let mut compiled_class = compile_class(class, symbols, reporter, objects)?;

        if let Some(ref superclass) = class.superclass {
            let superclass = &classes[&superclass.value];

            compiled_class
                .methods
                .extend(superclass.methods.clone().into_iter());
        }

        classes.insert(class.name, compiled_class);
    }

    Ok((
        Program {
            functions: funcs,
            classes,
        },
        objects,
    ))
}
