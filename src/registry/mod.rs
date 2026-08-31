//! Immutable Rivergate scenario definitions and validated lookup tables.
//!
//! Purpose: define the single shared `Registry` that all simulation,
//! bootstrap, command, and projection paths read, plus its deterministic
//! `fingerprint` that binds saves to the exact definitions they were built
//! against.
//! Owns: `ScenarioDef` / `DistrictDef` / `GoodDef` / `RecipeDef` /
//! `InstitutionDef`, dense 0..N typed-ID indexing, `guild_for_recipe`
//! trade→guild mapping, `build_rivergate_registry` assembly, and the
//! `DeterministicRegistryHasher` used for fingerprint and `SaveRevision`.
//! Reads: nothing (definitions are authored, not loaded).
//! Mutates: nothing after construction (builder pattern only during
//! `build`; afterwards `Registry` is `&`-shared).
//! Does not own: mutable campaign state, simulation rules, or persistence.
//! Canonical operations: `build_rivergate_registry()` → `get_*` / `get_*_id`
//! lookups → `fingerprint()` for save/load binding; builder `register_*`
//! apis with duplicate-key and reference validation.
//! Relevant invariants: dense 0..N indexing validated by `debug_assert!`;
//! every recipe and institution reference checked at build; `fingerprint`
//! covers all behavior-relevant defs in stable typed-ID order via FNV-1a;
//! guild mapping is exhaustive and tested.
//! Focused tests: `src/registry/mod.rs::tests` duplicate-key and reference
//! validation, chartered-guild coverage, key-resolution round-trip.

use crate::ids::{DistrictId, GoodId, InstitutionId, RecipeId};
use crate::money::{Money, Quantity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoodCategory {
    Staple,
    Drink,
    Textile,
    Fuel,
    Material,
    Tool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstitutionKind {
    CraftGuild,
    MerchantGuild,
    Council,
    Court,
    Watch,
    Treasury,
    Charity,
    MarketOffice,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioDef {
    key: String,
    name: String,
    start_year: i32,
}

impl ScenarioDef {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn start_year(&self) -> i32 {
        self.start_year
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistrictDef {
    id: DistrictId,
    key: String,
    name: String,
    population: u32,
    base_rent: Money,
}

impl DistrictDef {
    #[must_use]
    pub const fn id(&self) -> DistrictId {
        self.id
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn population(&self) -> u32 {
        self.population
    }

    #[must_use]
    pub const fn base_rent(&self) -> Money {
        self.base_rent
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodDef {
    id: GoodId,
    key: String,
    name: String,
    category: GoodCategory,
    base_price: Money,
    target_market_stock: Quantity,
    daily_spoilage_basis_points: u16,
}

impl GoodDef {
    #[must_use]
    pub const fn id(&self) -> GoodId {
        self.id
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn category(&self) -> GoodCategory {
        self.category
    }

    #[must_use]
    pub const fn base_price(&self) -> Money {
        self.base_price
    }

    #[must_use]
    pub const fn target_market_stock(&self) -> Quantity {
        self.target_market_stock
    }

    #[must_use]
    pub const fn daily_spoilage_basis_points(&self) -> u16 {
        self.daily_spoilage_basis_points
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeInput {
    good_id: GoodId,
    quantity: Quantity,
}

impl RecipeInput {
    #[must_use]
    pub const fn good_id(&self) -> GoodId {
        self.good_id
    }

    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeDef {
    id: RecipeId,
    key: String,
    name: String,
    inputs: Vec<RecipeInput>,
    output_good_id: GoodId,
    output_quantity: Quantity,
    daily_operating_cost: Money,
    administrative_load: u16,
}

impl RecipeDef {
    #[must_use]
    pub const fn id(&self) -> RecipeId {
        self.id
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn inputs(&self) -> &[RecipeInput] {
        &self.inputs
    }

    #[must_use]
    pub const fn output_good_id(&self) -> GoodId {
        self.output_good_id
    }

    #[must_use]
    pub const fn output_quantity(&self) -> Quantity {
        self.output_quantity
    }

    #[must_use]
    pub const fn daily_operating_cost(&self) -> Money {
        self.daily_operating_cost
    }

    #[must_use]
    pub const fn administrative_load(&self) -> u16 {
        self.administrative_load
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstitutionDef {
    id: InstitutionId,
    key: String,
    name: String,
    kind: InstitutionKind,
    district_id: DistrictId,
}

impl InstitutionDef {
    #[must_use]
    pub const fn id(&self) -> InstitutionId {
        self.id
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> InstitutionKind {
        self.kind
    }

    #[must_use]
    pub const fn district_id(&self) -> DistrictId {
        self.district_id
    }
}

#[derive(Clone, Debug)]
pub struct Registry {
    scenario: ScenarioDef,
    districts: Vec<DistrictDef>,
    goods: Vec<GoodDef>,
    recipes: Vec<RecipeDef>,
    institutions: Vec<InstitutionDef>,
    district_by_key: BTreeMap<String, DistrictId>,
    good_by_key: BTreeMap<String, GoodId>,
    recipe_by_key: BTreeMap<String, RecipeId>,
    institution_by_key: BTreeMap<String, InstitutionId>,
}

impl Registry {
    #[must_use]
    pub const fn scenario(&self) -> &ScenarioDef {
        &self.scenario
    }

    #[must_use]
    pub fn districts(&self) -> &[DistrictDef] {
        &self.districts
    }

    #[must_use]
    pub fn goods(&self) -> &[GoodDef] {
        &self.goods
    }

    #[must_use]
    pub fn recipes(&self) -> &[RecipeDef] {
        &self.recipes
    }

    #[must_use]
    pub fn institutions(&self) -> &[InstitutionDef] {
        &self.institutions
    }

    #[must_use]
    pub fn get_district(&self, id: DistrictId) -> Option<&DistrictDef> {
        let def = self.districts.get(id.value() as usize);
        debug_assert!(
            def.is_none_or(|def| def.id() == id),
            "district definitions are densely indexed by typed ID"
        );
        def
    }

    #[must_use]
    pub fn get_good(&self, id: GoodId) -> Option<&GoodDef> {
        let def = self.goods.get(id.value() as usize);
        debug_assert!(
            def.is_none_or(|def| def.id() == id),
            "good definitions are densely indexed by typed ID"
        );
        def
    }

    #[must_use]
    pub fn get_recipe(&self, id: RecipeId) -> Option<&RecipeDef> {
        let def = self.recipes.get(id.value() as usize);
        debug_assert!(
            def.is_none_or(|def| def.id() == id),
            "recipe definitions are densely indexed by typed ID"
        );
        def
    }

    #[must_use]
    pub fn get_institution(&self, id: InstitutionId) -> Option<&InstitutionDef> {
        let def = self.institutions.get(id.value() as usize);
        debug_assert!(
            def.is_none_or(|def| def.id() == id),
            "institution definitions are densely indexed by typed ID"
        );
        def
    }

    #[must_use]
    pub fn get_district_id(&self, key: &str) -> Option<DistrictId> {
        self.district_by_key.get(key).copied()
    }

    #[must_use]
    pub fn get_good_id(&self, key: &str) -> Option<GoodId> {
        self.good_by_key.get(key).copied()
    }

    #[must_use]
    pub fn get_recipe_id(&self, key: &str) -> Option<RecipeId> {
        self.recipe_by_key.get(key).copied()
    }

    #[must_use]
    pub fn get_institution_id(&self, key: &str) -> Option<InstitutionId> {
        self.institution_by_key.get(key).copied()
    }

    /// The chartered guild that governs a recipe's trade, if any. Every
    /// Rivergate craft and import trade answers to exactly one guild, so a
    /// business's recipe identifies the institution whose membership carries
    /// craft standing; unmapped recipes have no chartered trade.
    #[must_use]
    pub fn guild_for_recipe(&self, recipe_id: RecipeId) -> Option<InstitutionId> {
        let key = self.get_recipe(recipe_id)?.key();
        let institution_key = match key {
            "baking" | "milling" | "brewing" => "bakers_guild",
            "weaving" => "weavers_guild",
            "toolmaking" | "charcoal_burning" => "smiths_guild",
            "grain_import" | "wool_import" | "timber_import" | "iron_import" => "carters_guild",
            _ => return None,
        };
        self.get_institution_id(institution_key)
    }

    /// Computes a deterministic canonical fingerprint of the behavior-relevant registry definitions
    /// in stable typed-ID order.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = DeterministicRegistryHasher::new();
        hasher.write_str("scenario");
        hasher.write_str(self.scenario.key());
        hasher.write_str(self.scenario.name());
        hasher.write_i32(self.scenario.start_year());

        let mut districts: Vec<_> = self.districts.iter().collect();
        districts.sort_by_key(|d| d.id().value());
        for district in districts {
            hasher.write_str("district");
            hasher.write_u32(district.id().value());
            hasher.write_str(district.key());
            hasher.write_str(district.name());
            hasher.write_u32(district.population());
            hasher.write_i64(district.base_rent().copper());
        }

        let mut goods: Vec<_> = self.goods.iter().collect();
        goods.sort_by_key(|g| g.id().value());
        for good in goods {
            hasher.write_str("good");
            hasher.write_u32(good.id().value());
            hasher.write_str(good.key());
            hasher.write_str(good.name());
            let cat_byte = match good.category() {
                GoodCategory::Staple => 0_u8,
                GoodCategory::Drink => 1_u8,
                GoodCategory::Textile => 2_u8,
                GoodCategory::Fuel => 3_u8,
                GoodCategory::Material => 4_u8,
                GoodCategory::Tool => 5_u8,
            };
            hasher.write_u8(cat_byte);
            hasher.write_i64(good.base_price().copper());
            hasher.write_i64(good.target_market_stock().milliunits());
            hasher.write_u16(good.daily_spoilage_basis_points());
        }

        let mut recipes: Vec<_> = self.recipes.iter().collect();
        recipes.sort_by_key(|r| r.id().value());
        for recipe in recipes {
            hasher.write_str("recipe");
            hasher.write_u32(recipe.id().value());
            hasher.write_str(recipe.key());
            hasher.write_str(recipe.name());
            let mut inputs: Vec<_> = recipe.inputs().iter().collect();
            inputs.sort_by_key(|i| i.good_id().value());
            hasher.write_u32(u32::try_from(inputs.len()).unwrap_or(u32::MAX));
            for input in inputs {
                hasher.write_u32(input.good_id().value());
                hasher.write_i64(input.quantity().milliunits());
            }
            hasher.write_u32(recipe.output_good_id().value());
            hasher.write_i64(recipe.output_quantity().milliunits());
            hasher.write_i64(recipe.daily_operating_cost().copper());
            hasher.write_u16(recipe.administrative_load());
        }

        let mut institutions: Vec<_> = self.institutions.iter().collect();
        institutions.sort_by_key(|i| i.id().value());
        for institution in institutions {
            hasher.write_str("institution");
            hasher.write_u32(institution.id().value());
            hasher.write_str(institution.key());
            hasher.write_str(institution.name());
            let kind_byte = match institution.kind() {
                InstitutionKind::CraftGuild => 0_u8,
                InstitutionKind::MerchantGuild => 1_u8,
                InstitutionKind::Council => 2_u8,
                InstitutionKind::Court => 3_u8,
                InstitutionKind::Watch => 4_u8,
                InstitutionKind::Treasury => 5_u8,
                InstitutionKind::Charity => 6_u8,
                InstitutionKind::MarketOffice => 7_u8,
            };
            hasher.write_u8(kind_byte);
            hasher.write_u32(institution.district_id().value());
        }

        hasher.finish()
    }
}

#[derive(Default)]
pub(crate) struct DeterministicRegistryHasher {
    state: u64,
}

impl DeterministicRegistryHasher {
    pub(crate) const fn new() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub(crate) fn write_u8(&mut self, val: u8) {
        self.write(&[val]);
    }

    pub(crate) fn write_u16(&mut self, val: u16) {
        self.write(&val.to_le_bytes());
    }

    pub(crate) fn write_u32(&mut self, val: u32) {
        self.write(&val.to_le_bytes());
    }

    pub(crate) fn write_i32(&mut self, val: i32) {
        self.write(&val.to_le_bytes());
    }

    pub(crate) fn write_i64(&mut self, val: i64) {
        self.write(&val.to_le_bytes());
    }

    pub(crate) fn write_str(&mut self, s: &str) {
        self.write_u32(u32::try_from(s.len()).unwrap_or(u32::MAX));
        self.write(s.as_bytes());
    }

    pub(crate) const fn finish(&self) -> u64 {
        self.state
    }
}

#[derive(Debug)]
struct RegistryBuilder {
    scenario: ScenarioDef,
    districts: Vec<DistrictDef>,
    goods: Vec<GoodDef>,
    recipes: Vec<RecipeDef>,
    institutions: Vec<InstitutionDef>,
    district_by_key: BTreeMap<String, DistrictId>,
    good_by_key: BTreeMap<String, GoodId>,
    recipe_by_key: BTreeMap<String, RecipeId>,
    institution_by_key: BTreeMap<String, InstitutionId>,
}

impl RegistryBuilder {
    fn new(scenario: ScenarioDef) -> Self {
        Self {
            scenario,
            districts: Vec::new(),
            goods: Vec::new(),
            recipes: Vec::new(),
            institutions: Vec::new(),
            district_by_key: BTreeMap::new(),
            good_by_key: BTreeMap::new(),
            recipe_by_key: BTreeMap::new(),
            institution_by_key: BTreeMap::new(),
        }
    }

    fn register_district(
        &mut self,
        key: &str,
        name: &str,
        population: u32,
        base_rent: Money,
    ) -> DistrictId {
        assert!(
            !self.district_by_key.contains_key(key),
            "duplicate district key: {key}"
        );
        assert!(
            population > 0,
            "district {key} must have a positive population"
        );
        assert!(
            base_rent >= Money::ZERO,
            "district {key} must not have a negative base rent"
        );
        let id = DistrictId::new(u32::try_from(self.districts.len()).expect("too many districts"));
        self.districts.push(DistrictDef {
            id,
            key: key.to_owned(),
            name: name.to_owned(),
            population,
            base_rent,
        });
        self.district_by_key.insert(key.to_owned(), id);
        id
    }

    fn register_good(
        &mut self,
        key: &str,
        name: &str,
        category: GoodCategory,
        base_price: Money,
        target_market_stock: Quantity,
        daily_spoilage_basis_points: u16,
    ) -> GoodId {
        assert!(
            !self.good_by_key.contains_key(key),
            "duplicate good key: {key}"
        );
        assert!(
            base_price.copper() > 0,
            "good {key} must have a positive base price"
        );
        assert!(
            target_market_stock > Quantity::ZERO,
            "good {key} must have positive target stock"
        );
        assert!(
            daily_spoilage_basis_points <= 10_000,
            "good {key} has invalid spoilage"
        );
        let id = GoodId::new(u32::try_from(self.goods.len()).expect("too many goods"));
        self.goods.push(GoodDef {
            id,
            key: key.to_owned(),
            name: name.to_owned(),
            category,
            base_price,
            target_market_stock,
            daily_spoilage_basis_points,
        });
        self.good_by_key.insert(key.to_owned(), id);
        id
    }

    fn register_recipe(
        &mut self,
        key: &str,
        name: &str,
        inputs: Vec<(GoodId, Quantity)>,
        output: (GoodId, Quantity),
        daily_operating_cost: Money,
        administrative_load: u16,
    ) -> RecipeId {
        let (output_good_id, output_quantity) = output;
        assert!(
            !self.recipe_by_key.contains_key(key),
            "duplicate recipe key: {key}"
        );
        assert!(
            !output_quantity.is_negative() && !output_quantity.is_zero(),
            "recipe {key} has invalid output quantity"
        );
        assert!(
            daily_operating_cost.copper() >= 0,
            "recipe {key} has negative operating cost"
        );
        let id = RecipeId::new(u32::try_from(self.recipes.len()).expect("too many recipes"));
        let mut input_goods = BTreeSet::new();
        let inputs = inputs
            .into_iter()
            .map(|(good_id, quantity)| {
                assert!(
                    !quantity.is_negative() && !quantity.is_zero(),
                    "recipe {key} has invalid input quantity"
                );
                assert!(
                    input_goods.insert(good_id),
                    "recipe {key} repeats input good {good_id}"
                );
                RecipeInput { good_id, quantity }
            })
            .collect();
        self.recipes.push(RecipeDef {
            id,
            key: key.to_owned(),
            name: name.to_owned(),
            inputs,
            output_good_id,
            output_quantity,
            daily_operating_cost,
            administrative_load,
        });
        self.recipe_by_key.insert(key.to_owned(), id);
        id
    }

    fn register_institution(
        &mut self,
        key: &str,
        name: &str,
        kind: InstitutionKind,
        district_id: DistrictId,
    ) -> InstitutionId {
        assert!(
            !self.institution_by_key.contains_key(key),
            "duplicate institution key: {key}"
        );
        let id = InstitutionId::new(
            u32::try_from(self.institutions.len()).expect("too many institutions"),
        );
        self.institutions.push(InstitutionDef {
            id,
            key: key.to_owned(),
            name: name.to_owned(),
            kind,
            district_id,
        });
        self.institution_by_key.insert(key.to_owned(), id);
        id
    }

    fn build(self) -> Registry {
        let valid_districts: BTreeSet<_> = self.districts.iter().map(DistrictDef::id).collect();
        let valid_goods: BTreeSet<_> = self.goods.iter().map(GoodDef::id).collect();

        for recipe in &self.recipes {
            assert!(
                valid_goods.contains(&recipe.output_good_id),
                "recipe {} references missing output good {}",
                recipe.key,
                recipe.output_good_id
            );
            for input in &recipe.inputs {
                assert!(
                    valid_goods.contains(&input.good_id),
                    "recipe {} references missing input good {}",
                    recipe.key,
                    input.good_id
                );
            }
        }

        for institution in &self.institutions {
            assert!(
                valid_districts.contains(&institution.district_id),
                "institution {} references missing district {}",
                institution.key,
                institution.district_id
            );
        }

        Registry {
            scenario: self.scenario,
            districts: self.districts,
            goods: self.goods,
            recipes: self.recipes,
            institutions: self.institutions,
            district_by_key: self.district_by_key,
            good_by_key: self.good_by_key,
            recipe_by_key: self.recipe_by_key,
            institution_by_key: self.institution_by_key,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RivergateDistricts {
    old_quarter: DistrictId,
    riverside: DistrictId,
    market_ward: DistrictId,
    northgate: DistrictId,
    southern_reach: DistrictId,
    temple_hill: DistrictId,
}

#[derive(Clone, Copy, Debug)]
struct RivergateGoods {
    grain: GoodId,
    flour: GoodId,
    bread: GoodId,
    ale: GoodId,
    wool: GoodId,
    cloth: GoodId,
    timber: GoodId,
    charcoal: GoodId,
    iron: GoodId,
    tools: GoodId,
}

#[must_use]
pub fn build_rivergate_registry() -> Registry {
    let mut builder = RegistryBuilder::new(ScenarioDef {
        key: "rivergate".to_owned(),
        name: "Rivergate".to_owned(),
        start_year: 1325,
    });
    let districts = register_rivergate_districts(&mut builder);
    let goods = register_rivergate_goods(&mut builder);
    register_food_recipes(&mut builder, goods);
    register_textile_recipes(&mut builder, goods);
    register_material_recipes(&mut builder, goods);
    register_guild_institutions(&mut builder, districts);
    register_civic_institutions(&mut builder, districts);
    builder.build()
}

fn register_rivergate_districts(builder: &mut RegistryBuilder) -> RivergateDistricts {
    RivergateDistricts {
        old_quarter: builder.register_district(
            "old_quarter",
            "Old Quarter",
            7_200,
            Money::from_copper(180),
        ),
        riverside: builder.register_district(
            "riverside",
            "Riverside",
            8_800,
            Money::from_copper(130),
        ),
        market_ward: builder.register_district(
            "market_ward",
            "Market Ward",
            6_400,
            Money::from_copper(220),
        ),
        northgate: builder.register_district(
            "northgate",
            "Northgate",
            5_600,
            Money::from_copper(120),
        ),
        southern_reach: builder.register_district(
            "southern_reach",
            "Southern Reach",
            9_900,
            Money::from_copper(95),
        ),
        temple_hill: builder.register_district(
            "temple_hill",
            "Temple Hill",
            4_300,
            Money::from_copper(260),
        ),
    }
}

fn register_rivergate_goods(builder: &mut RegistryBuilder) -> RivergateGoods {
    RivergateGoods {
        grain: builder.register_good(
            "grain",
            "Grain",
            GoodCategory::Staple,
            Money::from_copper(8),
            Quantity::from_units(2_400),
            5,
        ),
        flour: builder.register_good(
            "flour",
            "Flour",
            GoodCategory::Staple,
            Money::from_copper(14),
            Quantity::from_units(1_200),
            12,
        ),
        bread: builder.register_good(
            "bread",
            "Bread",
            GoodCategory::Staple,
            Money::from_copper(22),
            Quantity::from_units(900),
            120,
        ),
        ale: builder.register_good(
            "ale",
            "Ale",
            GoodCategory::Drink,
            Money::from_copper(18),
            Quantity::from_units(700),
            35,
        ),
        wool: builder.register_good(
            "wool",
            "Wool",
            GoodCategory::Textile,
            Money::from_copper(26),
            Quantity::from_units(500),
            3,
        ),
        cloth: builder.register_good(
            "cloth",
            "Cloth",
            GoodCategory::Textile,
            Money::from_copper(54),
            Quantity::from_units(350),
            1,
        ),
        timber: builder.register_good(
            "timber",
            "Timber",
            GoodCategory::Material,
            Money::from_copper(20),
            Quantity::from_units(800),
            2,
        ),
        charcoal: builder.register_good(
            "charcoal",
            "Charcoal",
            GoodCategory::Fuel,
            Money::from_copper(28),
            Quantity::from_units(450),
            1,
        ),
        iron: builder.register_good(
            "iron",
            "Iron",
            GoodCategory::Material,
            Money::from_copper(48),
            Quantity::from_units(300),
            0,
        ),
        tools: builder.register_good(
            "tools",
            "Tools",
            GoodCategory::Tool,
            Money::from_copper(165),
            Quantity::from_units(260),
            0,
        ),
    }
}

fn register_food_recipes(builder: &mut RegistryBuilder, goods: RivergateGoods) {
    builder.register_recipe(
        "grain_import",
        "Western Grain Trade",
        Vec::new(),
        (goods.grain, Quantity::from_units(24)),
        Money::from_copper(90),
        8,
    );
    builder.register_recipe(
        "milling",
        "Milling",
        vec![(goods.grain, Quantity::from_units(10))],
        (goods.flour, Quantity::from_units(16)),
        Money::from_copper(60),
        6,
    );
    builder.register_recipe(
        "baking",
        "Bread Baking",
        vec![(goods.flour, Quantity::from_units(5))],
        (goods.bread, Quantity::from_units(10)),
        Money::from_copper(72),
        7,
    );
    builder.register_recipe(
        "brewing",
        "Ale Brewing",
        vec![(goods.grain, Quantity::from_units(6))],
        (goods.ale, Quantity::from_units(10)),
        Money::from_copper(68),
        7,
    );
}

fn register_textile_recipes(builder: &mut RegistryBuilder, goods: RivergateGoods) {
    builder.register_recipe(
        "wool_import",
        "Upland Wool Trade",
        Vec::new(),
        (goods.wool, Quantity::from_units(12)),
        Money::from_copper(110),
        9,
    );
    builder.register_recipe(
        "weaving",
        "Cloth Weaving",
        vec![(goods.wool, Quantity::from_units(6))],
        (goods.cloth, Quantity::from_units(6)),
        Money::from_copper(115),
        10,
    );
}

fn register_material_recipes(builder: &mut RegistryBuilder, goods: RivergateGoods) {
    builder.register_recipe(
        "timber_import",
        "Northern Timber Trade",
        Vec::new(),
        (goods.timber, Quantity::from_units(16)),
        Money::from_copper(95),
        8,
    );
    builder.register_recipe(
        "charcoal_burning",
        "Charcoal Burning",
        vec![(goods.timber, Quantity::from_units(8))],
        (goods.charcoal, Quantity::from_units(10)),
        Money::from_copper(55),
        6,
    );
    builder.register_recipe(
        "iron_import",
        "Valley Iron Trade",
        Vec::new(),
        (goods.iron, Quantity::from_units(10)),
        Money::from_copper(105),
        10,
    );
    builder.register_recipe(
        "toolmaking",
        "Toolmaking",
        vec![
            (goods.iron, Quantity::from_units(3)),
            (goods.charcoal, Quantity::from_units(2)),
        ],
        (goods.tools, Quantity::from_units(5)),
        Money::from_copper(85),
        10,
    );
}

fn register_guild_institutions(builder: &mut RegistryBuilder, districts: RivergateDistricts) {
    builder.register_institution(
        "bakers_guild",
        "Guild of Bakers",
        InstitutionKind::CraftGuild,
        districts.market_ward,
    );
    builder.register_institution(
        "weavers_guild",
        "Guild of Weavers",
        InstitutionKind::CraftGuild,
        districts.southern_reach,
    );
    builder.register_institution(
        "smiths_guild",
        "Guild of Smiths",
        InstitutionKind::CraftGuild,
        districts.northgate,
    );
    builder.register_institution(
        "carters_guild",
        "Guild of Carters",
        InstitutionKind::CraftGuild,
        districts.riverside,
    );
    builder.register_institution(
        "merchant_guild",
        "Merchant Guild",
        InstitutionKind::MerchantGuild,
        districts.market_ward,
    );
}

fn register_civic_institutions(builder: &mut RegistryBuilder, districts: RivergateDistricts) {
    builder.register_institution(
        "city_council",
        "Council of Rivergate",
        InstitutionKind::Council,
        districts.old_quarter,
    );
    builder.register_institution(
        "market_office",
        "Office of Markets",
        InstitutionKind::MarketOffice,
        districts.market_ward,
    );
    builder.register_institution(
        "civic_court",
        "Civic Court",
        InstitutionKind::Court,
        districts.old_quarter,
    );
    builder.register_institution(
        "city_watch",
        "Rivergate Watch",
        InstitutionKind::Watch,
        districts.northgate,
    );
    builder.register_institution(
        "treasury",
        "Civic Treasury",
        InstitutionKind::Treasury,
        districts.old_quarter,
    );
    builder.register_institution(
        "saint_oria_house",
        "House of Saint Oria",
        InstitutionKind::Charity,
        districts.temple_hill,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "repeats input good")]
    fn duplicate_recipe_inputs_are_rejected() {
        let mut builder = RegistryBuilder::new(ScenarioDef {
            key: "test".to_owned(),
            name: "Test".to_owned(),
            start_year: 1,
        });
        let good_id = builder.register_good(
            "input",
            "Input",
            GoodCategory::Material,
            Money::from_copper(1),
            Quantity::ONE,
            0,
        );

        builder.register_recipe(
            "duplicate",
            "Duplicate",
            vec![(good_id, Quantity::ONE), (good_id, Quantity::ONE)],
            (good_id, Quantity::ONE),
            Money::ZERO,
            1,
        );
    }

    #[test]
    fn rivergate_registry_resolves_every_registered_key() {
        let registry = build_rivergate_registry();

        for district in registry.districts() {
            assert_eq!(
                registry.get_district_id(district.key()),
                Some(district.id())
            );
        }
        for good in registry.goods() {
            assert_eq!(registry.get_good_id(good.key()), Some(good.id()));
        }
        for recipe in registry.recipes() {
            assert_eq!(registry.get_recipe_id(recipe.key()), Some(recipe.id()));
        }
        for institution in registry.institutions() {
            assert_eq!(
                registry.get_institution_id(institution.key()),
                Some(institution.id())
            );
        }
    }

    #[test]
    fn every_rivergate_trade_answers_to_one_chartered_guild() {
        let registry = build_rivergate_registry();

        for recipe in registry.recipes() {
            let guild = registry
                .guild_for_recipe(recipe.id())
                .unwrap_or_else(|| panic!("recipe {} must have a chartered guild", recipe.key()));
            let kind = registry
                .get_institution(guild)
                .expect("chartered guild must exist")
                .kind();
            assert!(
                matches!(
                    kind,
                    InstitutionKind::CraftGuild | InstitutionKind::MerchantGuild
                ),
                "recipe {} must map to a chartered guild, not {kind:?}",
                recipe.key()
            );
        }

        // An out-of-range recipe id has no chartered guild.
        let beyond = crate::ids::RecipeId::new(u32::from(
            u16::try_from(registry.recipes().len()).expect("recipe count must fit u16"),
        ));
        assert!(registry.get_recipe(beyond).is_none());
        assert!(registry.guild_for_recipe(beyond).is_none());
    }
}
