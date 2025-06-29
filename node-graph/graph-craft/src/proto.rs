use crate::document::{InlineRust, value};
use crate::document::{NodeId, OriginalLocation};
pub use graphene_core::registry::*;
use graphene_core::*;
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;

#[derive(Debug, Default, PartialEq, Clone, Hash, Eq, serde::Serialize, serde::Deserialize)]
/// A list of [`ProtoNode`]s, which is an intermediate step between the [`crate::document::NodeNetwork`] and the `BorrowTree` containing a single flattened network.
pub struct ProtoNetwork {
	// TODO: remove this since it seems to be unused?
	// Should a proto Network even allow inputs? Don't think so
	pub inputs: Vec<NodeId>,
	/// The node ID that provides the output. This node is then responsible for calling the rest of the graph.
	pub output: NodeId,
	/// A list of nodes stored in a Vec to allow for sorting.
	pub nodes: Vec<(NodeId, ProtoNode)>,
}

impl core::fmt::Display for ProtoNetwork {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		f.write_str("Proto Network with nodes: ")?;
		fn write_node(f: &mut core::fmt::Formatter<'_>, network: &ProtoNetwork, id: NodeId, indent: usize) -> core::fmt::Result {
			f.write_str(&"\t".repeat(indent))?;
			let Some((_, node)) = network.nodes.iter().find(|(node_id, _)| *node_id == id) else {
				return f.write_str("{{Unknown Node}}");
			};
			f.write_str("Node: ")?;
			f.write_str(&node.identifier.name)?;

			f.write_str("\n")?;
			f.write_str(&"\t".repeat(indent))?;
			f.write_str("{\n")?;

			f.write_str(&"\t".repeat(indent + 1))?;
			f.write_str("Input: ")?;
			match &node.input {
				ProtoNodeInput::None => f.write_str("None")?,
				ProtoNodeInput::ManualComposition(ty) => f.write_fmt(format_args!("Manual Composition (type = {ty:?})"))?,
				ProtoNodeInput::Node(_) => f.write_str("Node")?,
				ProtoNodeInput::NodeLambda(_) => f.write_str("Lambda Node")?,
			}
			f.write_str("\n")?;

			match &node.construction_args {
				ConstructionArgs::Value(value) => {
					f.write_str(&"\t".repeat(indent + 1))?;
					f.write_fmt(format_args!("Value construction argument: {value:?}"))?
				}
				ConstructionArgs::Nodes(nodes) => {
					for id in nodes {
						write_node(f, network, id.0, indent + 1)?;
					}
				}
				ConstructionArgs::Inline(inline) => {
					f.write_str(&"\t".repeat(indent + 1))?;
					f.write_fmt(format_args!("Inline construction argument: {inline:?}"))?
				}
			}
			f.write_str(&"\t".repeat(indent))?;
			f.write_str("}\n")?;
			Ok(())
		}

		let id = self.output;
		write_node(f, self, id, 0)
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// Defines the arguments used to construct the boxed node struct. This is used to call the constructor function in the `node_registry.rs` file - which is hidden behind a wall of macros.
pub enum ConstructionArgs {
	/// A value of a type that is known, allowing serialization (serde::Deserialize is not object safe)
	Value(MemoHash<value::TaggedValue>),
	/// A list of nodes used as inputs to the constructor function in `node_registry.rs`.
	/// The bool indicates whether to treat the node as lambda node.
	// TODO: use a struct for clearer naming.
	Nodes(Vec<(NodeId, bool)>),
	/// Used for GPU computation to work around the limitations of rust-gpu.
	Inline(InlineRust),
}

impl Eq for ConstructionArgs {}

impl PartialEq for ConstructionArgs {
	fn eq(&self, other: &Self) -> bool {
		match (&self, &other) {
			(Self::Nodes(n1), Self::Nodes(n2)) => n1 == n2,
			(Self::Value(v1), Self::Value(v2)) => v1 == v2,
			_ => {
				use std::hash::Hasher;
				let hash = |input: &Self| {
					let mut hasher = rustc_hash::FxHasher::default();
					input.hash(&mut hasher);
					hasher.finish()
				};
				hash(self) == hash(other)
			}
		}
	}
}

impl Hash for ConstructionArgs {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		core::mem::discriminant(self).hash(state);
		match self {
			Self::Nodes(nodes) => {
				for node in nodes {
					node.hash(state);
				}
			}
			Self::Value(value) => value.hash(state),
			Self::Inline(inline) => inline.hash(state),
		}
	}
}

impl ConstructionArgs {
	// TODO: what? Used in the gpu_compiler crate for something.
	pub fn new_function_args(&self) -> Vec<String> {
		match self {
			ConstructionArgs::Nodes(nodes) => nodes.iter().map(|(n, _)| format!("n{:0x}", n.0)).collect(),
			ConstructionArgs::Value(value) => vec![value.to_primitive_string()],
			ConstructionArgs::Inline(inline) => vec![inline.expr.clone()],
		}
	}
}

#[derive(Debug, Clone, PartialEq, Hash, Eq, serde::Serialize, serde::Deserialize)]
/// A proto node is an intermediate step between the `DocumentNode` and the boxed struct that actually runs the node (found in the [`BorrowTree`]).
/// At different stages in the compilation process, this struct will be transformed into a reduced (more restricted) form acting as a subset of its original form, but that restricted form is still valid in the earlier stage in the compilation process before it was transformed.
pub struct ProtoNode {
	pub construction_args: ConstructionArgs,
	pub input: ProtoNodeInput,
	pub identifier: ProtoNodeIdentifier,
	pub original_location: OriginalLocation,
	pub skip_deduplication: bool,
}

impl Default for ProtoNode {
	fn default() -> Self {
		Self {
			identifier: ProtoNodeIdentifier::new("graphene_core::ops::IdentityNode"),
			construction_args: ConstructionArgs::Value(value::TaggedValue::U32(0).into()),
			input: ProtoNodeInput::None,
			original_location: OriginalLocation::default(),
			skip_deduplication: false,
		}
	}
}

/// Similar to the document node's [`crate::document::NodeInput`].
#[derive(Debug, PartialEq, Eq, Clone, Hash, serde::Serialize, serde::Deserialize)]
pub enum ProtoNodeInput {
	/// This input will be converted to `()` as the call argument.
	None,
	/// A ManualComposition input represents an input that opts out of being resolved through the `ComposeNode`, which first runs the previous (upstream) node, then passes that evaluated
	/// result to this node. Instead, ManualComposition lets this node actually consume the provided input instead of passing it to its predecessor.
	///
	/// Say we have the network `a -> b -> c` where `c` is the output node and `a` is the input node.
	/// We would expect `a` to get input from the network, `b` to get input from `a`, and `c` to get input from `b`.
	/// This could be represented as `f(x) = c(b(a(x)))`. `a` is run with input `x` from the network. `b` is run with input from `a`. `c` is run with input from `b`.
	///
	/// However if `b`'s input is using manual composition, this means it would instead be `f(x) = c(b(x))`. This means that `b` actually gets input from the network, and `a` is not automatically
	/// executed as it would be using the default ComposeNode flow. Now `b` can use its own logic to decide when or if it wants to run `a` and how to use its output. For example, the CacheNode can
	/// look up `x` in its cache and return the result, or otherwise call `a`, cache the result, and return it.
	ManualComposition(Type),
	/// The previous node where automatic (not manual) composition occurs when compiled. The entire network, of which the node is the output, is fed as input.
	///
	/// Grayscale example:
	///
	/// We're interested in receiving an input of the desaturated image data which has been fed through a grayscale filter.
	/// (If we were interested in the grayscale filter itself, we would use the `NodeLambda` variant.)
	Node(NodeId),
	/// Unlike the `Node` variant, with `NodeLambda` we treat the connected node singularly as a lambda node while ignoring all nodes which feed into it from upstream.
	///
	/// Grayscale example:
	///
	/// We're interested in receiving an input of a particular image filter, such as a grayscale filter in the form of a grayscale node lambda.
	/// (If we were interested in some image data that had been fed through a grayscale filter, we would use the `Node` variant.)
	NodeLambda(NodeId),
}

impl ProtoNode {
	/// A stable node ID is a hash of a node that should stay constant. This is used in order to remove duplicates from the graph.
	/// In the case of `skip_deduplication`, the `document_node_path` is also hashed in order to avoid duplicate monitor nodes from being removed (which would make it impossible to load thumbnails).
	pub fn stable_node_id(&self) -> Option<NodeId> {
		use std::hash::Hasher;
		let mut hasher = rustc_hash::FxHasher::default();

		self.identifier.name.hash(&mut hasher);
		self.construction_args.hash(&mut hasher);
		if self.skip_deduplication {
			self.original_location.path.hash(&mut hasher);
		}

		std::mem::discriminant(&self.input).hash(&mut hasher);
		match self.input {
			ProtoNodeInput::None => (),
			ProtoNodeInput::ManualComposition(ref ty) => {
				ty.hash(&mut hasher);
			}
			ProtoNodeInput::Node(id) => (id, false).hash(&mut hasher),
			ProtoNodeInput::NodeLambda(id) => (id, true).hash(&mut hasher),
		};

		Some(NodeId(hasher.finish()))
	}

	/// Construct a new [`ProtoNode`] with the specified construction args and a `ClonedNode` implementation.
	pub fn value(value: ConstructionArgs, path: Vec<NodeId>) -> Self {
		let inputs_exposed = match &value {
			ConstructionArgs::Nodes(nodes) => nodes.len() + 1,
			_ => 2,
		};
		Self {
			identifier: ProtoNodeIdentifier::new("graphene_core::value::ClonedNode"),
			construction_args: value,
			input: ProtoNodeInput::ManualComposition(concrete!(Context)),
			original_location: OriginalLocation {
				path: Some(path),
				inputs_exposed: vec![false; inputs_exposed],
				..Default::default()
			},
			skip_deduplication: false,
		}
	}

	/// Converts all references to other node IDs into new IDs by running the specified function on them.
	/// This can be used when changing the IDs of the nodes, for example in the case of generating stable IDs.
	pub fn map_ids(&mut self, f: impl Fn(NodeId) -> NodeId, skip_lambdas: bool) {
		match self.input {
			ProtoNodeInput::Node(id) => self.input = ProtoNodeInput::Node(f(id)),
			ProtoNodeInput::NodeLambda(id) => {
				if !skip_lambdas {
					self.input = ProtoNodeInput::NodeLambda(f(id))
				}
			}
			_ => (),
		}

		if let ConstructionArgs::Nodes(ids) = &mut self.construction_args {
			ids.iter_mut().filter(|(_, lambda)| !(skip_lambdas && *lambda)).for_each(|(id, _)| *id = f(*id));
		}
	}

	pub fn unwrap_construction_nodes(&self) -> Vec<(NodeId, bool)> {
		match &self.construction_args {
			ConstructionArgs::Nodes(nodes) => nodes.clone(),
			_ => panic!("tried to unwrap nodes from non node construction args \n node: {self:#?}"),
		}
	}
}

#[derive(Clone, Copy, PartialEq)]
enum NodeState {
	Unvisited,
	Visiting,
	Visited,
}

impl ProtoNetwork {
	fn check_ref(&self, ref_id: &NodeId, id: &NodeId) {
		debug_assert!(
			self.nodes.iter().any(|(check_id, _)| check_id == ref_id),
			"Node id:{id} has a reference which uses node id:{ref_id} which doesn't exist in network {self:#?}"
		);
	}

	#[cfg(debug_assertions)]
	pub fn example() -> (Self, NodeId, ProtoNode) {
		let node_id = NodeId(1);
		let proto_node = ProtoNode::default();
		let proto_network = ProtoNetwork {
			inputs: vec![node_id],
			output: node_id,
			nodes: vec![(node_id, proto_node.clone())],
		};
		(proto_network, node_id, proto_node)
	}

	/// Construct a hashmap containing a list of the nodes that depend on this proto network.
	pub fn collect_outwards_edges(&self) -> HashMap<NodeId, Vec<NodeId>> {
		let mut edges: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
		for (id, node) in &self.nodes {
			match &node.input {
				ProtoNodeInput::Node(ref_id) | ProtoNodeInput::NodeLambda(ref_id) => {
					self.check_ref(ref_id, id);
					edges.entry(*ref_id).or_default().push(*id)
				}
				_ => (),
			}

			if let ConstructionArgs::Nodes(ref_nodes) = &node.construction_args {
				for (ref_id, _) in ref_nodes {
					self.check_ref(ref_id, id);
					edges.entry(*ref_id).or_default().push(*id)
				}
			}
		}
		edges
	}

	/// Convert all node IDs to be stable (based on the hash generated by [`ProtoNode::stable_node_id`]).
	/// This function requires that the graph be topologically sorted.
	pub fn generate_stable_node_ids(&mut self) {
		debug_assert!(self.is_topologically_sorted());
		let outwards_edges = self.collect_outwards_edges();

		for index in 0..self.nodes.len() {
			let Some(sni) = self.nodes[index].1.stable_node_id() else {
				panic!("failed to generate stable node id for node {:#?}", self.nodes[index].1);
			};
			self.replace_node_id(&outwards_edges, NodeId(index as u64), sni, false);
			self.nodes[index].0 = sni;
		}
	}

	// TODO: Remove
	/// Create a hashmap with the list of nodes this proto network depends on/uses as inputs.
	pub fn collect_inwards_edges(&self) -> HashMap<NodeId, Vec<NodeId>> {
		let mut edges: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
		for (id, node) in &self.nodes {
			match &node.input {
				ProtoNodeInput::Node(ref_id) | ProtoNodeInput::NodeLambda(ref_id) => {
					self.check_ref(ref_id, id);
					edges.entry(*id).or_default().push(*ref_id)
				}
				_ => (),
			}

			if let ConstructionArgs::Nodes(ref_nodes) = &node.construction_args {
				for (ref_id, _) in ref_nodes {
					self.check_ref(ref_id, id);
					edges.entry(*id).or_default().push(*ref_id)
				}
			}
		}
		edges
	}

	fn collect_inwards_edges_with_mapping(&self) -> (Vec<Vec<usize>>, FxHashMap<NodeId, usize>) {
		let id_map: FxHashMap<_, _> = self.nodes.iter().enumerate().map(|(idx, (id, _))| (*id, idx)).collect();

		// Collect inwards edges using dense indices
		let mut inwards_edges = vec![Vec::new(); self.nodes.len()];
		for (node_id, node) in &self.nodes {
			let node_index = id_map[node_id];
			match &node.input {
				ProtoNodeInput::Node(ref_id) | ProtoNodeInput::NodeLambda(ref_id) => {
					self.check_ref(ref_id, &NodeId(node_index as u64));
					inwards_edges[node_index].push(id_map[ref_id]);
				}
				_ => {}
			}

			if let ConstructionArgs::Nodes(ref_nodes) = &node.construction_args {
				for (ref_id, _) in ref_nodes {
					self.check_ref(ref_id, &NodeId(node_index as u64));
					inwards_edges[node_index].push(id_map[ref_id]);
				}
			}
		}

		(inwards_edges, id_map)
	}

	/// Inserts a [`structural::ComposeNode`] for each node that has a [`ProtoNodeInput::Node`]. The compose node evaluates the first node, and then sends the result into the second node.
	pub fn resolve_inputs(&mut self) -> Result<(), String> {
		// Perform topological sort once
		self.reorder_ids()?;

		let max_id = self.nodes.len() as u64 - 1;

		// Collect outward edges once
		let outwards_edges = self.collect_outwards_edges();

		// Iterate over nodes in topological order
		for node_id in 0..=max_id {
			let node_id = NodeId(node_id);

			let (_, node) = &mut self.nodes[node_id.0 as usize];

			if let ProtoNodeInput::Node(input_node_id) = node.input {
				// Create a new node that composes the current node and its input node
				let compose_node_id = NodeId(self.nodes.len() as u64);

				let (_, input_node_id_proto) = &self.nodes[input_node_id.0 as usize];

				let input = input_node_id_proto.input.clone();

				let mut path = input_node_id_proto.original_location.path.clone();
				if let Some(path) = &mut path {
					path.push(node_id);
				}

				self.nodes.push((
					compose_node_id,
					ProtoNode {
						identifier: ProtoNodeIdentifier::new("graphene_core::structural::ComposeNode"),
						construction_args: ConstructionArgs::Nodes(vec![(input_node_id, false), (node_id, true)]),
						input,
						original_location: OriginalLocation { path, ..Default::default() },
						skip_deduplication: false,
					},
				));

				self.replace_node_id(&outwards_edges, node_id, compose_node_id, true);
			}
		}
		self.reorder_ids()?;
		Ok(())
	}

	/// Update all of the references to a node ID in the graph with a new ID named `compose_node_id`.
	fn replace_node_id(&mut self, outwards_edges: &HashMap<NodeId, Vec<NodeId>>, node_id: NodeId, compose_node_id: NodeId, skip_lambdas: bool) {
		// Update references in other nodes to use the new compose node
		if let Some(referring_nodes) = outwards_edges.get(&node_id) {
			for &referring_node_id in referring_nodes {
				let (_, referring_node) = &mut self.nodes[referring_node_id.0 as usize];
				referring_node.map_ids(|id| if id == node_id { compose_node_id } else { id }, skip_lambdas)
			}
		}

		if self.output == node_id {
			self.output = compose_node_id;
		}

		self.inputs.iter_mut().for_each(|id| {
			if *id == node_id {
				*id = compose_node_id;
			}
		});
	}

	// Based on https://en.wikipedia.org/wiki/Topological_sorting#Depth-first_search
	// This approach excludes nodes that are not connected
	pub fn topological_sort(&self) -> Result<(Vec<NodeId>, FxHashMap<NodeId, usize>), String> {
		let (inwards_edges, id_map) = self.collect_inwards_edges_with_mapping();
		let mut sorted = Vec::with_capacity(self.nodes.len());
		let mut stack = vec![id_map[&self.output]];
		let mut state = vec![NodeState::Unvisited; self.nodes.len()];

		while let Some(&node_index) = stack.last() {
			match state[node_index] {
				NodeState::Unvisited => {
					state[node_index] = NodeState::Visiting;
					for &dep_index in inwards_edges[node_index].iter().rev() {
						match state[dep_index] {
							NodeState::Visiting => {
								return Err(format!("Cycle detected involving node {}", self.nodes[dep_index].0));
							}
							NodeState::Unvisited => {
								stack.push(dep_index);
							}
							NodeState::Visited => {}
						}
					}
				}
				NodeState::Visiting => {
					stack.pop();
					state[node_index] = NodeState::Visited;
					sorted.push(NodeId(node_index as u64));
				}
				NodeState::Visited => {
					stack.pop();
				}
			}
		}

		Ok((sorted, id_map))
	}

	fn is_topologically_sorted(&self) -> bool {
		let mut visited = HashSet::new();

		let inwards_edges = self.collect_inwards_edges();
		for (id, _) in &self.nodes {
			for &dependency in inwards_edges.get(id).unwrap_or(&Vec::new()) {
				if !visited.contains(&dependency) {
					dbg!(id, dependency);
					dbg!(&visited);
					dbg!(&self.nodes);
					return false;
				}
			}
			visited.insert(*id);
		}
		true
	}

	/// Sort the nodes vec so it is in a topological order. This ensures that no node takes an input from a node that is found later in the list.
	fn reorder_ids(&mut self) -> Result<(), String> {
		let (order, _id_map) = self.topological_sort()?;

		// // Map of node ids to their current index in the nodes vector
		// let current_positions: FxHashMap<_, _> = self.nodes.iter().enumerate().map(|(pos, (id, _))| (*id, pos)).collect();

		// // Map of node ids to their new index based on topological order
		let new_positions: FxHashMap<_, _> = order.iter().enumerate().map(|(pos, id)| (self.nodes[id.0 as usize].0, pos)).collect();
		// assert_eq!(id_map, current_positions);

		// Create a new nodes vector based on the topological order

		let mut new_nodes = Vec::with_capacity(order.len());
		for (index, &id) in order.iter().enumerate() {
			let mut node = std::mem::take(&mut self.nodes[id.0 as usize].1);
			// Update node references to reflect the new order
			node.map_ids(|id| NodeId(*new_positions.get(&id).expect("node not found in lookup table") as u64), false);
			new_nodes.push((NodeId(index as u64), node));
		}

		// Update node references to reflect the new order
		// new_nodes.iter_mut().for_each(|(_, node)| {
		// 	node.map_ids(|id| *new_positions.get(&id).expect("node not found in lookup table"), false);
		// });

		// Update the nodes vector and other references
		self.nodes = new_nodes;
		self.inputs = self.inputs.iter().filter_map(|id| new_positions.get(id).map(|x| NodeId(*x as u64))).collect();
		self.output = NodeId(*new_positions.get(&self.output).unwrap() as u64);

		assert_eq!(order.len(), self.nodes.len());
		Ok(())
	}
}
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GraphErrorType {
	NodeNotFound(NodeId),
	InputNodeNotFound(NodeId),
	UnexpectedGenerics { index: usize, inputs: Vec<Type> },
	NoImplementations,
	NoConstructor,
	InvalidImplementations { inputs: String, error_inputs: Vec<Vec<(usize, (Type, Type))>> },
	MultipleImplementations { inputs: String, valid: Vec<NodeIOTypes> },
}
impl Debug for GraphErrorType {
	// TODO: format with the document graph context so the input index is the same as in the graph UI.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			GraphErrorType::NodeNotFound(id) => write!(f, "Input node {id} is not present in the typing context"),
			GraphErrorType::InputNodeNotFound(id) => write!(f, "Input node {id} is not present in the typing context"),
			GraphErrorType::UnexpectedGenerics { index, inputs } => write!(f, "Generic inputs should not exist but found at {index}: {inputs:?}"),
			GraphErrorType::NoImplementations => write!(f, "No implementations found"),
			GraphErrorType::NoConstructor => write!(f, "No construct found for node"),
			GraphErrorType::InvalidImplementations { inputs, error_inputs } => {
				let format_error = |(index, (found, expected)): &(usize, (Type, Type))| {
					let index = index + 1;
					format!(
						"\
						• Input {index}:\n\
						…found:       {found}\n\
						…expected: {expected}\
						"
					)
				};
				let format_error_list = |errors: &Vec<(usize, (Type, Type))>| errors.iter().map(format_error).collect::<Vec<_>>().join("\n");
				let mut errors = error_inputs.iter().map(format_error_list).collect::<Vec<_>>();
				errors.sort();
				let errors = errors.join("\n");
				let incompatibility = if errors.chars().filter(|&c| c == '•').count() == 1 {
					"This input type is incompatible:"
				} else {
					"These input types are incompatible:"
				};

				write!(
					f,
					"\
					{incompatibility}\n\
					{errors}\n\
					\n\
					The node is currently receiving all of the following input types:\n\
					{inputs}\n\
					This is not a supported arrangement of types for the node.\
					"
				)
			}
			GraphErrorType::MultipleImplementations { inputs, valid } => write!(f, "Multiple implementations found ({inputs}):\n{valid:#?}"),
		}
	}
}
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphError {
	pub node_path: Vec<NodeId>,
	pub identifier: Cow<'static, str>,
	pub error: GraphErrorType,
}
impl GraphError {
	pub fn new(node: &ProtoNode, text: impl Into<GraphErrorType>) -> Self {
		Self {
			node_path: node.original_location.path.clone().unwrap_or_default(),
			identifier: node.identifier.name.clone(),
			error: text.into(),
		}
	}
}
impl Debug for GraphError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("NodeGraphError")
			.field("path", &self.node_path.iter().map(|id| id.0).collect::<Vec<_>>())
			.field("identifier", &self.identifier.to_string())
			.field("error", &self.error)
			.finish()
	}
}
pub type GraphErrors = Vec<GraphError>;

/// The `TypingContext` is used to store the types of the nodes indexed by their stable node id.
#[derive(Default, Clone, dyn_any::DynAny)]
pub struct TypingContext {
	lookup: Cow<'static, HashMap<ProtoNodeIdentifier, HashMap<NodeIOTypes, NodeConstructor>>>,
	inferred: HashMap<NodeId, NodeIOTypes>,
	constructor: HashMap<NodeId, NodeConstructor>,
}

impl TypingContext {
	/// Creates a new `TypingContext` with the given lookup table.
	pub fn new(lookup: &'static HashMap<ProtoNodeIdentifier, HashMap<NodeIOTypes, NodeConstructor>>) -> Self {
		Self {
			lookup: Cow::Borrowed(lookup),
			..Default::default()
		}
	}

	/// Updates the `TypingContext` with a given proto network. This will infer the types of the nodes
	/// and store them in the `inferred` field. The proto network has to be topologically sorted
	/// and contain fully resolved stable node ids.
	pub fn update(&mut self, network: &ProtoNetwork) -> Result<(), GraphErrors> {
		for (id, node) in network.nodes.iter() {
			self.infer(*id, node)?;
		}

		Ok(())
	}

	pub fn remove_inference(&mut self, node_id: NodeId) -> Option<NodeIOTypes> {
		self.constructor.remove(&node_id);
		self.inferred.remove(&node_id)
	}

	/// Returns the node constructor for a given node id.
	pub fn constructor(&self, node_id: NodeId) -> Option<NodeConstructor> {
		self.constructor.get(&node_id).copied()
	}

	/// Returns the type of a given node id if it exists
	pub fn type_of(&self, node_id: NodeId) -> Option<&NodeIOTypes> {
		self.inferred.get(&node_id)
	}

	/// Returns the inferred types for a given node id.
	pub fn infer(&mut self, node_id: NodeId, node: &ProtoNode) -> Result<NodeIOTypes, GraphErrors> {
		// Return the inferred type if it is already known
		if let Some(inferred) = self.inferred.get(&node_id) {
			return Ok(inferred.clone());
		}

		let inputs = match node.construction_args {
			// If the node has a value input we can infer the return type from it
			ConstructionArgs::Value(ref v) => {
				assert!(matches!(node.input, ProtoNodeInput::None) || matches!(node.input, ProtoNodeInput::ManualComposition(ref x) if x == &concrete!(Context)));
				// TODO: This should return a reference to the value
				let types = NodeIOTypes::new(concrete!(Context), Type::Future(Box::new(v.ty())), vec![]);
				self.inferred.insert(node_id, types.clone());
				return Ok(types);
			}
			// If the node has nodes as inputs we can infer the types from the node outputs
			ConstructionArgs::Nodes(ref nodes) => nodes
				.iter()
				.map(|(id, _)| {
					self.inferred
						.get(id)
						.ok_or_else(|| vec![GraphError::new(node, GraphErrorType::NodeNotFound(*id))])
						.map(|node| node.ty())
				})
				.collect::<Result<Vec<Type>, GraphErrors>>()?,
			ConstructionArgs::Inline(ref inline) => vec![inline.ty.clone()],
		};

		// Get the node input type from the proto node declaration
		// TODO: When removing automatic composition, rename this to just `call_argument`
		let primary_input_or_call_argument = match node.input {
			ProtoNodeInput::None => concrete!(()),
			ProtoNodeInput::ManualComposition(ref ty) => ty.clone(),
			ProtoNodeInput::Node(id) | ProtoNodeInput::NodeLambda(id) => {
				let input = self.inferred.get(&id).ok_or_else(|| vec![GraphError::new(node, GraphErrorType::InputNodeNotFound(id))])?;
				input.return_value.clone()
			}
		};
		let using_manual_composition = matches!(node.input, ProtoNodeInput::ManualComposition(_) | ProtoNodeInput::None);
		let impls = self.lookup.get(&node.identifier).ok_or_else(|| vec![GraphError::new(node, GraphErrorType::NoImplementations)])?;

		if let Some(index) = inputs.iter().position(|p| {
			matches!(p,
			Type::Fn(_, b) if matches!(b.as_ref(), Type::Generic(_)))
		}) {
			return Err(vec![GraphError::new(node, GraphErrorType::UnexpectedGenerics { index, inputs })]);
		}

		/// Checks if a proposed input to a particular (primary or secondary) input connector is valid for its type signature.
		/// `from` indicates the value given to a input, `to` indicates the input's allowed type as specified by its type signature.
		fn valid_type(from: &Type, to: &Type) -> bool {
			match (from, to) {
				// Direct comparison of two concrete types.
				(Type::Concrete(type1), Type::Concrete(type2)) => type1 == type2,
				// Check inner type for futures
				(Type::Future(type1), Type::Future(type2)) => valid_type(type1, type2),
				// Direct comparison of two function types.
				// Note: in the presence of subtyping, functions are considered on a "greater than or equal to" basis of its function type's generality.
				// That means we compare their types with a contravariant relationship, which means that a more general type signature may be substituted for a more specific type signature.
				// For example, we allow `T -> V` to be substituted with `T' -> V` or `() -> V` where T' and () are more specific than T.
				// This allows us to supply anything to a function that is satisfied with `()`.
				// In other words, we are implementing these two relations, where the >= operator means that the left side is more general than the right side:
				// - `T >= T' ⇒ (T' -> V) >= (T -> V)` (functions are contravariant in their input types)
				// - `V >= V' ⇒ (T -> V) >= (T -> V')` (functions are covariant in their output types)
				// While these two relations aren't a truth about the universe, they are a design decision that we are employing in our language design that is also common in other languages.
				// For example, Rust implements these same relations as it describes here: <https://doc.rust-lang.org/nomicon/subtyping.html>
				// Graphite doesn't have subtyping currently, but it used to have it, and may do so again, so we make sure to compare types in this way to make things easier.
				// More details explained here: <https://github.com/GraphiteEditor/Graphite/issues/1741>
				(Type::Fn(in1, out1), Type::Fn(in2, out2)) => valid_type(out2, out1) && valid_type(in1, in2),
				// If either the proposed input or the allowed input are generic, we allow the substitution (meaning this is a valid subtype).
				// TODO: Add proper generic counting which is not based on the name
				(Type::Generic(_), _) | (_, Type::Generic(_)) => true,
				// Reject unknown type relationships.
				_ => false,
			}
		}

		// List of all implementations that match the input types
		let valid_output_types = impls
			.keys()
			.filter(|node_io| valid_type(&node_io.call_argument, &primary_input_or_call_argument) && inputs.iter().zip(node_io.inputs.iter()).all(|(p1, p2)| valid_type(p1, p2)))
			.collect::<Vec<_>>();

		// Attempt to substitute generic types with concrete types and save the list of results
		let substitution_results = valid_output_types
			.iter()
			.map(|node_io| {
				let generics_lookup: Result<HashMap<_, _>, _> = collect_generics(node_io)
					.iter()
					.map(|generic| check_generic(node_io, &primary_input_or_call_argument, &inputs, generic).map(|x| (generic.to_string(), x)))
					.collect();

				generics_lookup.map(|generics_lookup| {
					let orig_node_io = (*node_io).clone();
					let mut new_node_io = orig_node_io.clone();
					replace_generics(&mut new_node_io, &generics_lookup);
					(new_node_io, orig_node_io)
				})
			})
			.collect::<Vec<_>>();

		// Collect all substitutions that are valid
		let valid_impls = substitution_results.iter().filter_map(|result| result.as_ref().ok()).collect::<Vec<_>>();

		match valid_impls.as_slice() {
			[] => {
				let mut best_errors = usize::MAX;
				let mut error_inputs = Vec::new();
				for node_io in impls.keys() {
					let current_errors = [&primary_input_or_call_argument]
						.into_iter()
						.chain(&inputs)
						.cloned()
						.zip([&node_io.call_argument].into_iter().chain(&node_io.inputs).cloned())
						.enumerate()
						.filter(|(_, (p1, p2))| !valid_type(p1, p2))
						.map(|(index, ty)| {
							let i = node.original_location.inputs(index).min_by_key(|s| s.node.len()).map(|s| s.index).unwrap_or(index);
							let i = if using_manual_composition { i } else { i + 1 };
							(i, ty)
						})
						.collect::<Vec<_>>();
					if current_errors.len() < best_errors {
						best_errors = current_errors.len();
						error_inputs.clear();
					}
					if current_errors.len() <= best_errors {
						error_inputs.push(current_errors);
					}
				}
				let inputs = [&primary_input_or_call_argument]
					.into_iter()
					.chain(&inputs)
					.enumerate()
					// TODO: Make the following line's if statement conditional on being a call argument or primary input
					.filter_map(|(i, t)| {
						let i = if using_manual_composition { i } else { i + 1 };
						if i == 0 { None } else { Some(format!("• Input {i}: {t}")) }
					})
					.collect::<Vec<_>>()
					.join("\n");
				Err(vec![GraphError::new(node, GraphErrorType::InvalidImplementations { inputs, error_inputs })])
			}
			[(node_io, org_nio)] => {
				let node_io = node_io.clone();

				// Save the inferred type
				self.inferred.insert(node_id, node_io.clone());
				self.constructor.insert(node_id, impls[org_nio]);
				Ok(node_io)
			}
			// If two types are available and one of them accepts () an input, always choose that one
			[first, second] => {
				if first.0.call_argument != second.0.call_argument {
					for (node_io, orig_nio) in [first, second] {
						if node_io.call_argument != concrete!(()) {
							continue;
						}

						// Save the inferred type
						self.inferred.insert(node_id, node_io.clone());
						self.constructor.insert(node_id, impls[orig_nio]);
						return Ok(node_io.clone());
					}
				}
				let inputs = [&primary_input_or_call_argument].into_iter().chain(&inputs).map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
				let valid = valid_output_types.into_iter().cloned().collect();
				Err(vec![GraphError::new(node, GraphErrorType::MultipleImplementations { inputs, valid })])
			}

			_ => {
				let inputs = [&primary_input_or_call_argument].into_iter().chain(&inputs).map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
				let valid = valid_output_types.into_iter().cloned().collect();
				Err(vec![GraphError::new(node, GraphErrorType::MultipleImplementations { inputs, valid })])
			}
		}
	}
}

/// Returns a list of all generic types used in the node
fn collect_generics(types: &NodeIOTypes) -> Vec<Cow<'static, str>> {
	let inputs = [&types.call_argument].into_iter().chain(types.inputs.iter().map(|x| x.nested_type()));
	let mut generics = inputs
		.filter_map(|t| match t {
			Type::Generic(out) => Some(out.clone()),
			_ => None,
		})
		.collect::<Vec<_>>();
	if let Type::Generic(out) = &types.return_value {
		generics.push(out.clone());
	}
	generics.dedup();
	generics
}

/// Checks if a generic type can be substituted with a concrete type and returns the concrete type
fn check_generic(types: &NodeIOTypes, input: &Type, parameters: &[Type], generic: &str) -> Result<Type, String> {
	let inputs = [(Some(&types.call_argument), Some(input))]
		.into_iter()
		.chain(types.inputs.iter().map(|x| x.fn_input()).zip(parameters.iter().map(|x| x.fn_input())))
		.chain(types.inputs.iter().map(|x| x.fn_output()).zip(parameters.iter().map(|x| x.fn_output())));
	let concrete_inputs = inputs.filter(|(ni, _)| matches!(ni, Some(Type::Generic(input)) if generic == input));
	let mut outputs = concrete_inputs.flat_map(|(_, out)| out);
	let out_ty = outputs
		.next()
		.ok_or_else(|| format!("Generic output type {generic} is not dependent on input {input:?} or parameters {parameters:?}",))?;
	if outputs.any(|ty| ty != out_ty) {
		return Err(format!("Generic output type {generic} is dependent on multiple inputs or parameters",));
	}
	Ok(out_ty.clone())
}

/// Returns a list of all generic types used in the node
fn replace_generics(types: &mut NodeIOTypes, lookup: &HashMap<String, Type>) {
	let replace = |ty: &Type| {
		let Type::Generic(ident) = ty else {
			return None;
		};
		lookup.get(ident.as_ref()).cloned()
	};
	types.call_argument.replace_nested(replace);
	types.return_value.replace_nested(replace);
	for input in &mut types.inputs {
		input.replace_nested(replace);
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use crate::proto::{ConstructionArgs, ProtoNetwork, ProtoNode, ProtoNodeInput};

	#[test]
	fn topological_sort() {
		let construction_network = test_network();
		let (sorted, _) = construction_network.topological_sort().expect("Error when calling 'topological_sort' on 'construction_network.");
		let sorted: Vec<_> = sorted.iter().map(|x| construction_network.nodes[x.0 as usize].0).collect();
		println!("{sorted:#?}");
		assert_eq!(sorted, vec![NodeId(14), NodeId(10), NodeId(11), NodeId(1)]);
	}

	#[test]
	fn topological_sort_with_cycles() {
		let construction_network = test_network_with_cycles();
		let sorted = construction_network.topological_sort();

		assert!(sorted.is_err())
	}

	#[test]
	fn id_reordering() {
		let mut construction_network = test_network();
		construction_network.reorder_ids().expect("Error when calling 'reorder_ids' on 'construction_network.");
		let (sorted, _) = construction_network.topological_sort().expect("Error when calling 'topological_sort' on 'construction_network.");
		let sorted: Vec<_> = sorted.iter().map(|x| construction_network.nodes[x.0 as usize].0).collect();
		println!("nodes: {:#?}", construction_network.nodes);
		assert_eq!(sorted, vec![NodeId(0), NodeId(1), NodeId(2), NodeId(3)]);
		let ids: Vec<_> = construction_network.nodes.iter().map(|(id, _)| *id).collect();
		println!("{ids:#?}");
		println!("nodes: {:#?}", construction_network.nodes);
		assert_eq!(construction_network.nodes[0].1.identifier.name.as_ref(), "value");
		assert_eq!(ids, vec![NodeId(0), NodeId(1), NodeId(2), NodeId(3)]);
	}

	#[test]
	fn id_reordering_idempotent() {
		let mut construction_network = test_network();
		construction_network.reorder_ids().expect("Error when calling 'reorder_ids' on 'construction_network.");
		construction_network.reorder_ids().expect("Error when calling 'reorder_ids' on 'construction_network.");
		let (sorted, _) = construction_network.topological_sort().expect("Error when calling 'topological_sort' on 'construction_network.");
		assert_eq!(sorted, vec![NodeId(0), NodeId(1), NodeId(2), NodeId(3)]);
		let ids: Vec<_> = construction_network.nodes.iter().map(|(id, _)| *id).collect();
		println!("{ids:#?}");
		assert_eq!(construction_network.nodes[0].1.identifier.name.as_ref(), "value");
		assert_eq!(ids, vec![NodeId(0), NodeId(1), NodeId(2), NodeId(3)]);
	}

	#[test]
	fn input_resolution() {
		let mut construction_network = test_network();
		construction_network.resolve_inputs().expect("Error when calling 'resolve_inputs' on 'construction_network.");
		println!("{construction_network:#?}");
		assert_eq!(construction_network.nodes[0].1.identifier.name.as_ref(), "value");
		assert_eq!(construction_network.nodes.len(), 6);
		assert_eq!(construction_network.nodes[5].1.construction_args, ConstructionArgs::Nodes(vec![(NodeId(3), false), (NodeId(4), true)]));
	}

	#[test]
	fn stable_node_id_generation() {
		let mut construction_network = test_network();
		construction_network.resolve_inputs().expect("Error when calling 'resolve_inputs' on 'construction_network.");
		construction_network.generate_stable_node_ids();
		assert_eq!(construction_network.nodes[0].1.identifier.name.as_ref(), "value");
		let ids: Vec<_> = construction_network.nodes.iter().map(|(id, _)| *id).collect();
		assert_eq!(
			ids,
			vec![
				NodeId(16997244687192517417),
				NodeId(12226224850522777131),
				NodeId(9162113827627229771),
				NodeId(12793582657066318419),
				NodeId(16945623684036608820),
				NodeId(2640415155091892458)
			]
		);
	}

	fn test_network() -> ProtoNetwork {
		ProtoNetwork {
			inputs: vec![NodeId(10)],
			output: NodeId(1),
			nodes: [
				(
					NodeId(7),
					ProtoNode {
						identifier: "id".into(),
						input: ProtoNodeInput::Node(NodeId(11)),
						construction_args: ConstructionArgs::Nodes(vec![]),
						..Default::default()
					},
				),
				(
					NodeId(1),
					ProtoNode {
						identifier: "id".into(),
						input: ProtoNodeInput::Node(NodeId(11)),
						construction_args: ConstructionArgs::Nodes(vec![]),
						..Default::default()
					},
				),
				(
					NodeId(10),
					ProtoNode {
						identifier: "cons".into(),
						input: ProtoNodeInput::ManualComposition(concrete!(u32)),
						construction_args: ConstructionArgs::Nodes(vec![(NodeId(14), false)]),
						..Default::default()
					},
				),
				(
					NodeId(11),
					ProtoNode {
						identifier: "add".into(),
						input: ProtoNodeInput::Node(NodeId(10)),
						construction_args: ConstructionArgs::Nodes(vec![]),
						..Default::default()
					},
				),
				(
					NodeId(14),
					ProtoNode {
						identifier: "value".into(),
						input: ProtoNodeInput::None,
						construction_args: ConstructionArgs::Value(value::TaggedValue::U32(2).into()),
						..Default::default()
					},
				),
			]
			.into_iter()
			.collect(),
		}
	}

	fn test_network_with_cycles() -> ProtoNetwork {
		ProtoNetwork {
			inputs: vec![NodeId(1)],
			output: NodeId(1),
			nodes: [
				(
					NodeId(1),
					ProtoNode {
						identifier: "id".into(),
						input: ProtoNodeInput::Node(NodeId(2)),
						construction_args: ConstructionArgs::Nodes(vec![]),
						..Default::default()
					},
				),
				(
					NodeId(2),
					ProtoNode {
						identifier: "id".into(),
						input: ProtoNodeInput::Node(NodeId(1)),
						construction_args: ConstructionArgs::Nodes(vec![]),
						..Default::default()
					},
				),
			]
			.into_iter()
			.collect(),
		}
	}
}
