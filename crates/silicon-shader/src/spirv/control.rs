//! Bounded, acyclic structured selections. Each branch keeps its own SSA/local definitions.
use super::*;

#[derive(Clone)]
struct Environment {
    available: BTreeSet<u32>,
    locals: BTreeMap<u32, Typed>,
    written: BTreeSet<u8>,
}
struct Block<'a> {
    code: Vec<&'a Op>,
    predecessors: BTreeSet<u32>,
}
type Path = (u32, Environment);

impl Compiler<'_> {
    fn environment(&self) -> Environment {
        Environment {
            available: self.available.clone().unwrap(),
            locals: self.locals.clone(),
            written: self.written.clone(),
        }
    }
    fn restore(&mut self, e: &Environment) {
        self.available = Some(e.available.clone());
        self.locals = e.locals.clone();
        self.written = e.written.clone();
    }
    fn path_value(&self, id: u32, e: &Environment) -> Result<Typed> {
        if !self.globals.contains(&id) && !e.available.contains(&id) {
            return Err(format!(
                "Phi value %{id} is unavailable on its predecessor path"
            ));
        }
        self.values
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("undefined Phi value %{id}"))
    }
    fn merge_value(&mut self, a: &Typed, b: &Typed) -> Result<Typed> {
        if a.ty != b.ty {
            return Err("selection input type mismatch".into());
        }
        self.value_lanes(a.ty)?;
        let (Value::Reg(x, xu), Value::Reg(y, yu)) = (&a.value, &b.value) else {
            return Err("selection values must be float/vector/bool registers".into());
        };
        let r = if x == y {
            *x
        } else {
            self.emit(|dst| Sir::Merge { dst, a: *x, b: *y })?
        };
        Ok(Typed {
            ty: a.ty,
            value: Value::Reg(r, *xu && *yu),
        })
    }
    fn join_paths(&mut self, paths: &[Path]) -> Result<()> {
        self.restore(&paths[0].1);
        if let Some((_, b)) = paths.get(1) {
            let a = &paths[0].1;
            self.available = Some(a.available.intersection(&b.available).copied().collect());
            self.written = a.written.intersection(&b.written).copied().collect();
            self.locals.clear();
            for (&id, v) in &a.locals {
                if let Some(w) = b.locals.get(&id) {
                    let merged = self.merge_value(v, w)?;
                    self.locals.insert(id, merged);
                }
            }
        }
        Ok(())
    }
    fn lower_phi(&mut self, op: &Op, paths: &[Path], predecessors: &BTreeSet<u32>) -> Result<()> {
        let a = &op.operands;
        let pairs: BTreeMap<_, _> = a[2..].chunks_exact(2).map(|p| (p[1], p[0])).collect();
        let actual: BTreeSet<_> = paths.iter().map(|(id, _)| *id).collect();
        if pairs.len() * 2 != a.len() - 2
            || pairs.keys().copied().collect::<BTreeSet<_>>() != *predecessors
            || actual != *predecessors
        {
            return Err("Phi requires exactly one value for every reachable predecessor".into());
        }
        self.value_lanes(a[0])?;
        let mut values = Vec::new();
        for (pred, e) in paths {
            let v = self.path_value(pairs[pred], e)?;
            if v.ty != a[0] || !matches!(v.value, Value::Reg(..)) {
                return Err("Phi input/result type mismatch".into());
            }
            values.push(v);
        }
        let v = if values.len() == 2 {
            self.merge_value(&values[0], &values[1])?
        } else {
            values
                .into_iter()
                .next()
                .ok_or("Phi without a predecessor")?
        };
        self.values.insert(a[1], v);
        self.available.as_mut().unwrap().insert(a[1]);
        Ok(())
    }
    fn check_return(&self) -> Result<()> {
        if self.stage == Stage::Compute {
            return self.written.contains(&0).then_some(()).ok_or_else(|| {
                "compute return must write its storage output on every live path".into()
            });
        }
        if !self.written.contains(&0) {
            return Err(
                "Return must write Position (vertex) or location 0 (fragment) on every live path"
                    .into(),
            );
        }
        for &loc in self.outputs.keys() {
            if !self
                .written
                .contains(&(loc + u8::from(self.stage == Stage::Vertex)))
            {
                return Err(format!(
                    "output location {loc} is not written on every returning path"
                ));
            }
        }
        Ok(())
    }
    pub(super) fn lower_control(&mut self, body: &[Op]) -> Result<()> {
        if body.is_empty() || body.len() > 4096 || body[0].opcode != 248 {
            return Err("main requires 1..4096 instructions starting with a Label".into());
        }
        let mut blocks: BTreeMap<u32, Block<'_>> = BTreeMap::new();
        let entry = body[0].operands[0];
        let mut current = entry;
        for op in body {
            if op.opcode == 248 {
                current = op.operands[0];
                blocks.insert(
                    current,
                    Block {
                        code: Vec::new(),
                        predecessors: BTreeSet::new(),
                    },
                );
            } else {
                blocks.get_mut(&current).unwrap().code.push(op);
            }
        }
        let mut edges = Vec::new();
        for (&id, block) in &blocks {
            let end = block.code.last().ok_or("empty basic block")?;
            if !matches!(end.opcode, 249 | 250 | 252 | 253 | 255) {
                return Err(end.error("basic block must end in Branch, BranchConditional, Kill, Return or Unreachable"));
            }
            let mut executable = false;
            for (i, op) in block.code.iter().enumerate() {
                if matches!(op.opcode, 249 | 250 | 252 | 253 | 255) && i + 1 != block.code.len() {
                    return Err(op.error("instruction after block terminator"));
                }
                if op.opcode == 59 {
                    if id != entry || executable || op.operands[2] != 7 {
                        return Err(op.error("Function variables must start the first block"));
                    }
                } else if op.opcode != 0 {
                    executable = true;
                }
                if op.opcode == 247
                    && (i + 2 != block.code.len() || end.opcode != 250 || op.operands[1] != 0)
                {
                    return Err(
                        op.error("SelectionMerge None must immediately precede BranchConditional")
                    );
                }
            }
            if end.opcode == 249 {
                edges.push((id, end.operands[0]));
            }
            if end.opcode == 250 {
                edges.extend(end.operands[1..].iter().map(|&to| (id, to)));
                if end.operands[1] == end.operands[2] {
                    return Err(end.error("conditional targets must be distinct"));
                }
            }
        }
        for (from, to) in edges {
            blocks
                .get_mut(&to)
                .ok_or_else(|| format!("branch target %{to} is not a main Label"))?
                .predecessors
                .insert(from);
        }
        if !blocks[&entry].predecessors.is_empty() {
            return Err("main entry has a predecessor".into());
        }
        let mut visited = BTreeSet::new();
        self.region(entry, None, Vec::new(), 0, &blocks, &mut visited)?;
        for (id, block) in &blocks {
            if !visited.contains(id)
                && (block.code.len() != 1
                    || block.code[0].opcode != 255
                    || !block.predecessors.is_empty())
            {
                return Err(format!(
                    "unreachable or overlapping block %{id} is outside the acyclic selection subset"
                ));
            }
        }
        Ok(())
    }
    fn region(
        &mut self,
        mut id: u32,
        stop: Option<u32>,
        mut incoming: Vec<Path>,
        depth: usize,
        blocks: &BTreeMap<u32, Block<'_>>,
        visited: &mut BTreeSet<u32>,
    ) -> Result<Option<Path>> {
        if depth > 64 {
            return Err("SPIR-V selection nesting exceeds 64".into());
        }
        loop {
            if Some(id) == stop {
                return Ok(incoming.into_iter().next());
            }
            let block = blocks
                .get(&id)
                .ok_or_else(|| format!("missing basic block %{id}"))?;
            if incoming
                .iter()
                .map(|(pred, _)| *pred)
                .collect::<BTreeSet<_>>()
                != block.predecessors
            {
                return Err(format!(
                    "block %{id} has predecessors outside its structured region"
                ));
            }
            if !visited.insert(id) {
                return Err(format!("cyclic or overlapping control flow at block %{id}"));
            }
            let mut prefix = true;
            for op in &block.code[..block.code.len() - 1] {
                match op.opcode {
                    245 if prefix => self
                        .lower_phi(op, &incoming, &block.predecessors)
                        .map_err(|e| op.error(e))?,
                    245 => return Err(op.error("Phi must start its block")),
                    247 => {
                        prefix = false;
                    }
                    0 => {}
                    12
                    | 59
                    | 61
                    | 62
                    | 65
                    | 79..=83
                    | 87
                    | 88
                    | 112
                    | 127
                    | 129
                    | 131
                    | 133
                    | 136
                    | 142
                    | 145
                    | 148
                    | 164..=169
                    | 180
                    | 182..=184
                    | 186
                    | 188
                    | 190 => {
                        prefix = false;
                        self.instruction(op).map_err(|e| op.error(e))?;
                        if let Some(result) = op.ids()?.0 {
                            self.available.as_mut().unwrap().insert(result);
                        }
                    }
                    _ => return Err(op.error("unsupported instruction in main block")),
                }
            }
            let end = block.code.last().unwrap();
            match end.opcode {
                249 => {
                    incoming = vec![(id, self.environment())];
                    id = end.operands[0];
                }
                250 => {
                    let merge = block
                        .code
                        .get(block.code.len().wrapping_sub(2))
                        .filter(|op| op.opcode == 247)
                        .ok_or_else(|| end.error("BranchConditional requires SelectionMerge"))?
                        .operands[0];
                    if !blocks.contains_key(&merge)
                        || merge == id
                        || Some(merge) == stop
                        || visited.contains(&merge)
                    {
                        return Err(
                            end.error("selection merge must be a distinct unvisited main Label")
                        );
                    }
                    let (condition, ty, _) = self.reg(end.operands[0]).map_err(|e| end.error(e))?;
                    if self.ty(ty)? != Ty::Bool {
                        return Err(end.error("BranchConditional requires scalar bool"));
                    }
                    let entry = self.environment();
                    self.ops.push(Sir::If { condition });
                    let yes = self.region(
                        end.operands[1],
                        Some(merge),
                        vec![(id, entry.clone())],
                        depth + 1,
                        blocks,
                        visited,
                    )?;
                    self.ops.push(Sir::Else);
                    self.restore(&entry);
                    let no = self.region(
                        end.operands[2],
                        Some(merge),
                        vec![(id, entry)],
                        depth + 1,
                        blocks,
                        visited,
                    )?;
                    self.ops.push(Sir::EndIf);
                    incoming = yes.into_iter().chain(no).collect();
                    if incoming.is_empty() {
                        return Ok(None);
                    }
                    self.join_paths(&incoming)?;
                    id = merge;
                }
                252 => {
                    if self.stage != Stage::Fragment {
                        return Err(end.error("Kill is only allowed in fragment shaders"));
                    }
                    self.ops.push(Sir::Discard);
                    return Ok(None);
                }
                253 => {
                    self.check_return().map_err(|e| end.error(e))?;
                    self.ops.push(Sir::Return);
                    return Ok(None);
                }
                _ => return Err(end.error("reachable Unreachable terminator")),
            }
        }
    }
}
