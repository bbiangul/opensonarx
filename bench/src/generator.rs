use sha2::{Digest, Sha256};

use crate::dataset::{BenchmarkCase, BenchmarkDataset, CaseMetadata, Difficulty, RelevantDoc};
use crate::image_gen;

// 20 topics
const TOPICS: &[&str] = &[
    "technology",
    "science",
    "health",
    "finance",
    "law",
    "education",
    "environment",
    "energy",
    "agriculture",
    "transportation",
    "manufacturing",
    "telecommunications",
    "entertainment",
    "sports",
    "politics",
    "real_estate",
    "cybersecurity",
    "ai_ml",
    "blockchain",
    "space",
];

const CONTENT_TYPES: &[&str] = &["article", "docs", "forum", "blog"];
const REGIONS: &[&str] = &["us", "eu", "asia", "global"];

/// Deterministic u64 from a seed string using SHA-256.
fn hash_seed(seed: &str) -> u64 {
    let hash = Sha256::digest(seed.as_bytes());
    u64::from_le_bytes(hash[0..8].try_into().unwrap())
}

/// Simple deterministic PRNG (xorshift64).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn usize_range(&mut self, lo: usize, hi: usize) -> usize {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() as usize) % (hi - lo)
    }

    fn pick<'a>(&mut self, items: &'a [&str]) -> &'a str {
        items[self.next_u64() as usize % items.len()]
    }
}

// ---------------------------------------------------------------------------
// Template data: each topic has 5+ query-doc pairs per difficulty level.
// (query, title, content)
// ---------------------------------------------------------------------------

struct TopicTemplates {
    topic: &'static str,
    easy: &'static [(&'static str, &'static str, &'static str)],
    medium: &'static [(&'static str, &'static str, &'static str)],
    hard: &'static [(&'static str, &'static str, &'static str)],
}

fn all_templates() -> Vec<TopicTemplates> {
    vec![
        TopicTemplates {
            topic: "technology",
            easy: &[
                (
                    "rust borrow checker",
                    "Rust Borrow Checker Explained",
                    "The Rust borrow checker enforces ownership rules at compile time, preventing data races and ensuring memory safety without a garbage collector.",
                ),
                (
                    "linux kernel modules",
                    "Writing Linux Kernel Modules",
                    "Linux kernel modules are pieces of code that can be loaded into the kernel on demand. They extend the functionality of the kernel without rebooting the system.",
                ),
                (
                    "git merge conflicts",
                    "Resolving Git Merge Conflicts",
                    "Git merge conflicts occur when two branches modify the same line of a file. You must manually resolve conflicts by editing the affected files and choosing which changes to keep.",
                ),
                (
                    "docker container networking",
                    "Docker Container Networking Guide",
                    "Docker provides several networking drivers including bridge, host, and overlay. Bridge is the default and creates a private internal network on the host.",
                ),
                (
                    "ssh key authentication",
                    "Setting Up SSH Key Authentication",
                    "SSH key authentication uses a pair of cryptographic keys: a public key stored on the server and a private key kept on your local machine for secure passwordless login.",
                ),
            ],
            medium: &[
                (
                    "memory safe systems language",
                    "Ownership and Borrowing in Rust",
                    "Rust prevents use-after-free bugs through its ownership model. Each value has a single owner, and references must not outlive the data they point to.",
                ),
                (
                    "self-contained deployable images",
                    "Container Image Best Practices",
                    "Container images package application code with all dependencies into a single artifact. Minimizing layer count and using multi-stage builds reduces image size.",
                ),
                (
                    "distributed version control workflow",
                    "Branch-Based Development with Git",
                    "Modern software teams use feature branches to isolate work. Pull requests enable code review before merging changes into the main branch.",
                ),
                (
                    "passwordless remote server access",
                    "Public Key Infrastructure for SSH",
                    "Asymmetric cryptography enables authentication without transmitting passwords. The server verifies a challenge signed by the client's private key.",
                ),
                (
                    "isolated dependency management",
                    "Managing Python Project Dependencies",
                    "Dependency isolation prevents version conflicts between projects. Tools like Poetry and pip-tools generate lock files to ensure reproducible builds.",
                ),
            ],
            hard: &[
                (
                    "preventing dangling pointers at compile time",
                    "Static Lifetime Analysis in Systems Programming",
                    "The type system statically guarantees that references never outlive their referents. By encoding lifetime relationships in function signatures, the compiler rejects programs that would cause undefined behavior.",
                ),
                (
                    "reproducible build artifacts across environments",
                    "Hermetic Build Systems and Reproducibility",
                    "Achieving byte-identical outputs requires controlling all inputs including toolchain versions, environment variables, and filesystem ordering. Content-addressable storage enables caching without invalidation bugs.",
                ),
                (
                    "coordinating parallel development streams",
                    "Trunk-Based Development at Scale",
                    "High-performing teams commit to the main branch frequently using short-lived feature flags. Continuous integration validates every change, reducing integration risk and enabling rapid delivery.",
                ),
                (
                    "proving identity without shared secrets",
                    "Zero-Knowledge Authentication Protocols",
                    "Challenge-response protocols allow a prover to demonstrate knowledge of a secret without revealing it. Elliptic curve cryptography provides strong security with compact key sizes.",
                ),
                (
                    "avoiding dependency hell in large codebases",
                    "Monorepo Dependency Management Strategies",
                    "Unified dependency graphs across a monorepo prevent version skew. Semantic versioning combined with lock files ensures that upgrades are intentional and tested.",
                ),
            ],
        },
        TopicTemplates {
            topic: "science",
            easy: &[
                (
                    "CRISPR gene editing",
                    "CRISPR-Cas9 Gene Editing Technology",
                    "CRISPR-Cas9 allows scientists to edit DNA sequences with high precision. The Cas9 protein acts as molecular scissors, cutting DNA at a location specified by a guide RNA.",
                ),
                (
                    "black hole event horizon",
                    "Understanding Black Hole Event Horizons",
                    "The event horizon is the boundary around a black hole beyond which nothing can escape, not even light. Its radius is determined by the Schwarzschild equation.",
                ),
                (
                    "quantum entanglement experiments",
                    "Recent Quantum Entanglement Research",
                    "Quantum entanglement links particles so that measuring one instantly affects the other regardless of distance. Bell test experiments have confirmed this non-local correlation.",
                ),
                (
                    "photosynthesis light reactions",
                    "The Light Reactions of Photosynthesis",
                    "During the light reactions of photosynthesis, chlorophyll absorbs photons and transfers energy through the electron transport chain to produce ATP and NADPH.",
                ),
                (
                    "plate tectonics continental drift",
                    "Plate Tectonics and Continental Drift Theory",
                    "Plate tectonics describes the movement of lithospheric plates on the asthenosphere. Convection currents in the mantle drive plate motion, causing earthquakes and volcanic activity.",
                ),
            ],
            medium: &[
                (
                    "programmable molecular scissors for DNA",
                    "Targeted Genome Modification Systems",
                    "Guide RNA directs an endonuclease to a complementary sequence in the genome. The resulting double-strand break is repaired by cellular machinery, enabling precise insertions or deletions.",
                ),
                (
                    "gravitational point of no return",
                    "Singularity Boundaries in General Relativity",
                    "At a critical radius surrounding a collapsed stellar remnant, spacetime curvature becomes so extreme that all future-directed paths lead inward. This defines the causal boundary of the compact object.",
                ),
                (
                    "spooky action at a distance",
                    "Non-Local Correlations in Quantum Mechanics",
                    "Measurements on spatially separated particles show correlations that exceed classical limits. The violation of Bell inequalities rules out local hidden variable theories.",
                ),
                (
                    "how plants convert sunlight to energy",
                    "Electron Transport in Chloroplast Thylakoids",
                    "Pigment molecules absorb electromagnetic radiation and funnel excitation energy to reaction centers. Water molecules are split to replace lost electrons, releasing oxygen as a byproduct.",
                ),
                (
                    "why continents move over time",
                    "Mantle Convection and Lithospheric Motion",
                    "Heat from radioactive decay drives convection cells in the asthenosphere. These currents exert drag on overlying plates, causing them to diverge, converge, or slide past each other.",
                ),
            ],
            hard: &[
                (
                    "rewriting cellular instruction sets with precision",
                    "Off-Target Effects in Genome Engineering",
                    "Engineered nucleases occasionally cleave unintended genomic loci with sequence similarity to the target. Whole-genome sequencing and improved guide design algorithms minimize these collateral modifications.",
                ),
                (
                    "information paradox at gravitational boundaries",
                    "Hawking Radiation and Unitarity in Quantum Gravity",
                    "Semiclassical calculations predict thermal emission from compact objects, suggesting information loss. Recent developments in holographic duality and the island formula may preserve unitarity.",
                ),
                (
                    "correlations that defy classical explanation",
                    "Contextuality and Non-Classicality Beyond Bell",
                    "The Kochen-Specker theorem demonstrates that measurement outcomes cannot be pre-assigned independently of context. This contextuality is a resource for quantum computational advantage.",
                ),
                (
                    "converting electromagnetic radiation into chemical bonds",
                    "Proton Motive Force and Carbon Fixation Coupling",
                    "The electrochemical gradient across thylakoid membranes drives ATP synthase rotation. The resulting ATP powers the Calvin cycle, converting inorganic carbon into glyceraldehyde-3-phosphate.",
                ),
                (
                    "geological evidence for ancient supercontinent cycles",
                    "Paleomagnetic Records and Rodinia Reconstruction",
                    "Remnant magnetization in ancient rocks records the latitude and orientation of continental blocks at the time of formation. Statistical analysis of pole positions constrains Neoproterozoic paleogeography.",
                ),
            ],
        },
        TopicTemplates {
            topic: "health",
            easy: &[
                (
                    "type 2 diabetes management",
                    "Managing Type 2 Diabetes",
                    "Type 2 diabetes management involves blood sugar monitoring, medication such as metformin, regular exercise, and dietary changes to maintain healthy glucose levels.",
                ),
                (
                    "COVID-19 mRNA vaccine mechanism",
                    "How mRNA Vaccines Work Against COVID-19",
                    "mRNA vaccines deliver genetic instructions for cells to produce spike protein. The immune system recognizes the spike protein as foreign and builds antibodies against it.",
                ),
                (
                    "cardiovascular exercise heart health",
                    "Cardiovascular Exercise and Heart Health Benefits",
                    "Regular cardiovascular exercise strengthens the heart muscle, lowers blood pressure, improves cholesterol levels, and reduces the risk of coronary artery disease.",
                ),
                (
                    "antibiotic resistance bacteria",
                    "Antibiotic Resistance in Bacteria",
                    "Antibiotic resistance occurs when bacteria evolve mechanisms to survive antimicrobial drugs. Overuse and misuse of antibiotics accelerate the spread of resistant strains.",
                ),
                (
                    "vitamin D deficiency symptoms",
                    "Signs and Symptoms of Vitamin D Deficiency",
                    "Vitamin D deficiency can cause fatigue, bone pain, muscle weakness, and mood changes. Risk factors include limited sun exposure, darker skin, and inadequate dietary intake.",
                ),
            ],
            medium: &[
                (
                    "controlling blood sugar without insulin injections",
                    "Oral Hypoglycemic Agents and Lifestyle Interventions",
                    "First-line treatment for insulin resistance focuses on reducing hepatic glucose output with biguanides. Dietary carbohydrate restriction and structured physical activity improve peripheral glucose uptake.",
                ),
                (
                    "teaching immune cells to recognize viral proteins",
                    "Nucleoside-Modified mRNA Immunization Platforms",
                    "Lipid nanoparticles deliver modified nucleoside transcripts to antigen-presenting cells. Translated surface antigens elicit robust humoral and cellular immune responses.",
                ),
                (
                    "strengthening the heart through regular movement",
                    "Aerobic Conditioning and Cardiac Remodeling",
                    "Sustained moderate-intensity exertion increases stroke volume through eccentric ventricular hypertrophy. Improved endothelial function reduces systemic vascular resistance over time.",
                ),
                (
                    "bacteria becoming immune to medicines",
                    "Mechanisms of Antimicrobial Resistance Evolution",
                    "Horizontal gene transfer via plasmids spreads resistance cassettes between bacterial species. Efflux pumps, enzymatic degradation, and target modification are common resistance strategies.",
                ),
                (
                    "health effects of insufficient sunlight exposure",
                    "Calcidiol Insufficiency and Musculoskeletal Health",
                    "Inadequate cutaneous synthesis of cholecalciferol impairs intestinal calcium absorption. Prolonged insufficiency leads to secondary hyperparathyroidism and reduced bone mineral density.",
                ),
            ],
            hard: &[
                (
                    "metabolic syndrome reversal through lifestyle",
                    "Adipokine Signaling and Insulin Sensitization Pathways",
                    "Exercise-induced myokine secretion modulates hepatic lipid metabolism via AMPK activation. Reduced visceral adiposity restores adiponectin levels, improving whole-body insulin sensitivity.",
                ),
                (
                    "programming adaptive immunity against novel pathogens",
                    "Germinal Center Dynamics Following Nucleic Acid Vaccination",
                    "Follicular helper T cells drive somatic hypermutation in B cell clones within germinal centers. Affinity maturation over weeks produces high-avidity neutralizing antibodies and long-lived plasma cells.",
                ),
                (
                    "how sustained exertion reshapes cardiac tissue",
                    "Exercise-Induced Cardiac Plasticity and Electrophysiological Adaptation",
                    "Chronic endurance training increases vagal tone, lowering resting heart rate. Atrial remodeling may predispose to arrhythmia in susceptible individuals despite overall cardiovascular benefit.",
                ),
                (
                    "horizontal spread of drug resistance genes",
                    "Integron-Mediated Resistance Cassette Acquisition in Gram-Negatives",
                    "Class 1 integrons capture gene cassettes via site-specific recombination at attI sites. Co-selection on multi-drug resistance plasmids accelerates dissemination across ecological boundaries.",
                ),
                (
                    "bone weakening from chronic nutrient shortfall",
                    "Osteoclast-Osteoblast Coupling in Secondary Hyperparathyroidism",
                    "Elevated parathyroid hormone stimulates RANKL expression on osteoblasts, promoting osteoclast differentiation. Net bone resorption exceeds formation, increasing fracture risk in cortical and trabecular compartments.",
                ),
            ],
        },
        TopicTemplates {
            topic: "finance",
            easy: &[
                (
                    "compound interest savings account",
                    "How Compound Interest Works in Savings",
                    "Compound interest is calculated on the initial principal and accumulated interest from previous periods, causing savings to grow exponentially over time.",
                ),
                (
                    "stock market index funds",
                    "Investing in Stock Market Index Funds",
                    "Index funds track a market benchmark like the S&P 500, providing broad diversification at low cost. They consistently outperform most actively managed funds over long periods.",
                ),
                (
                    "federal reserve interest rates",
                    "Federal Reserve Interest Rate Decisions",
                    "The Federal Reserve sets the federal funds rate to influence borrowing costs across the economy. Raising rates slows inflation while lowering rates stimulates economic growth.",
                ),
                (
                    "credit score factors FICO",
                    "Factors That Affect Your FICO Credit Score",
                    "FICO credit scores range from 300 to 850 and are determined by payment history, credit utilization, length of credit history, new credit inquiries, and credit mix.",
                ),
                (
                    "401k retirement plan contributions",
                    "Maximizing Your 401k Retirement Contributions",
                    "A 401k plan allows employees to save for retirement with pre-tax contributions. Employer matching contributions provide an immediate return on investment and should be maximized.",
                ),
            ],
            medium: &[
                (
                    "exponential growth of reinvested earnings",
                    "Time Value of Money and Compounding Frequency",
                    "The effective annual rate increases with compounding frequency. Continuous compounding represents the theoretical upper limit, calculated as the principal times e raised to the rate-time product.",
                ),
                (
                    "passive broad market equity exposure",
                    "Market-Capitalization Weighted Portfolio Construction",
                    "Cap-weighted portfolios allocate proportionally to each constituent's market value. This self-rebalancing approach minimizes turnover and captures the full equity risk premium.",
                ),
                (
                    "central bank monetary policy tools",
                    "Open Market Operations and Reserve Requirements",
                    "The central bank influences money supply through asset purchases and sales. Forward guidance shapes market expectations about the future path of short-term interest rates.",
                ),
                (
                    "numerical measure of borrower reliability",
                    "Statistical Models of Consumer Default Probability",
                    "Logistic regression models estimate the likelihood of loan delinquency based on historical payment patterns, outstanding balances, and account age. Score cutoffs determine credit approval thresholds.",
                ),
                (
                    "tax-advantaged retirement savings vehicles",
                    "Defined Contribution Plan Optimization Strategies",
                    "Pre-tax salary deferrals reduce current taxable income while growing tax-deferred. Roth contributions use after-tax dollars but provide tax-free withdrawals in retirement.",
                ),
            ],
            hard: &[
                (
                    "mathematical basis of wealth accumulation over time",
                    "Stochastic Calculus in Portfolio Growth Optimization",
                    "The Kelly criterion maximizes expected logarithmic utility of terminal wealth. Geometric Brownian motion models incorporate drift and volatility to derive optimal allocation fractions.",
                ),
                (
                    "systematic harvest of market risk premia",
                    "Factor Investing and Cross-Sectional Return Predictability",
                    "Fama-French factors decompose equity returns into market, size, and value components. Momentum and profitability factors capture additional cross-sectional variation unexplained by the three-factor model.",
                ),
                (
                    "managing economy-wide price stability",
                    "Taylor Rule Deviations and Unconventional Monetary Policy",
                    "When the zero lower bound constrains conventional tools, quantitative easing expands the central bank balance sheet. Yield curve control targets specific maturities to anchor long-term borrowing costs.",
                ),
                (
                    "predicting individual loan default likelihood",
                    "Machine Learning Approaches to Credit Risk Scoring",
                    "Gradient-boosted decision trees capture non-linear interactions between applicant features. Model interpretability tools like SHAP values help meet regulatory requirements for adverse action notices.",
                ),
                (
                    "optimizing lifetime tax burden on retirement assets",
                    "Roth Conversion Ladders and Marginal Tax Rate Arbitrage",
                    "Converting traditional IRA balances during low-income years locks in favorable marginal rates. Careful sequencing of withdrawals across account types minimizes the aggregate tax wedge over a multi-decade horizon.",
                ),
            ],
        },
        TopicTemplates {
            topic: "law",
            easy: &[
                (
                    "contract breach remedies",
                    "Legal Remedies for Breach of Contract",
                    "When a party fails to fulfill contractual obligations, the non-breaching party may seek damages, specific performance, or contract rescission as legal remedies.",
                ),
                (
                    "intellectual property patent filing",
                    "How to File a Patent Application",
                    "Patent applications require a detailed description, claims defining the invention's scope, and drawings. The USPTO examines applications for novelty, non-obviousness, and utility.",
                ),
                (
                    "fourth amendment search seizure",
                    "Fourth Amendment Search and Seizure Rights",
                    "The Fourth Amendment protects against unreasonable searches and seizures. Law enforcement generally needs a warrant supported by probable cause to search a person's property.",
                ),
                (
                    "tort law negligence elements",
                    "Elements of Negligence in Tort Law",
                    "To prove negligence, a plaintiff must establish duty of care, breach of that duty, causation linking the breach to harm, and actual damages suffered.",
                ),
                (
                    "employment discrimination workplace",
                    "Workplace Employment Discrimination Laws",
                    "Federal laws prohibit employment discrimination based on race, color, religion, sex, national origin, age, disability, and genetic information under Title VII and related statutes.",
                ),
            ],
            medium: &[
                (
                    "compensation when agreements are violated",
                    "Expectation Damages and the Benefit of the Bargain",
                    "Courts award expectation damages to place the injured party in the position they would have occupied had the contract been performed. Consequential damages require foreseeability at the time of contracting.",
                ),
                (
                    "protecting novel inventions through legal monopoly",
                    "Prosecution Strategy for Utility Patent Claims",
                    "Patent claims define the metes and bounds of legal protection. Broad independent claims maximize coverage while dependent claims provide fallback positions during prosecution.",
                ),
                (
                    "constitutional limits on government investigation",
                    "Warrant Requirements and the Exclusionary Rule",
                    "Evidence obtained through unconstitutional state action is inadmissible under the fruit of the poisonous tree doctrine. Exceptions include good faith, inevitable discovery, and independent source.",
                ),
                (
                    "proving someone failed their duty of care",
                    "Proximate Cause and Foreseeability in Negligence Actions",
                    "The but-for test establishes cause in fact, while proximate cause limits liability to foreseeable consequences. Intervening superseding causes may break the causal chain.",
                ),
                (
                    "unfair treatment of workers based on identity",
                    "Disparate Impact Analysis Under Title VII",
                    "Statistical evidence demonstrating that a facially neutral employment practice disproportionately affects a protected class establishes a prima facie case. The employer must show business necessity.",
                ),
            ],
            hard: &[
                (
                    "making an injured party whole after broken promises",
                    "Reliance Interest and Promissory Estoppel in Quasi-Contract",
                    "When a contract is unenforceable but the promisee has detrimentally relied on the promise, equity may impose liability. Restitutionary recovery prevents unjust enrichment of the breaching party.",
                ),
                (
                    "boundaries of exclusive rights over innovations",
                    "Claim Construction and the Doctrine of Equivalents",
                    "Markman hearings resolve claim term meaning as a matter of law. The doctrine of equivalents extends protection to insubstantial variations that perform substantially the same function in the same way.",
                ),
                (
                    "tension between privacy rights and public safety",
                    "Third-Party Doctrine and Digital Privacy in the Post-Carpenter Era",
                    "The Supreme Court held that cell-site location information is protected by the Fourth Amendment despite being held by third parties. Lower courts are extending this reasoning to other digital records.",
                ),
                (
                    "chain of responsibility for harm caused by inaction",
                    "Nonfeasance Liability and the Special Relationship Exception",
                    "The general rule imposes no duty to rescue absent a special relationship. Exceptions arise from voluntary assumption of duty, creation of peril, or statutory mandates in professional contexts.",
                ),
                (
                    "systemic bias embedded in neutral workplace rules",
                    "Griggs Legacy and the Business Necessity Defense Evolution",
                    "The Civil Rights Act of 1991 codified disparate impact theory and shifted the burden of proving business necessity to the employer. Validation studies under EEOC guidelines establish job-relatedness.",
                ),
            ],
        },
        TopicTemplates {
            topic: "education",
            easy: &[
                (
                    "online learning platforms MOOCs",
                    "Online Learning Platforms and MOOCs",
                    "Massive Open Online Courses provide free or low-cost education from top universities. Platforms like Coursera, edX, and Khan Academy offer courses in hundreds of subjects.",
                ),
                (
                    "standardized testing SAT scores",
                    "Understanding SAT Standardized Testing",
                    "The SAT measures reading, writing, and math skills for college admissions. Scores range from 400 to 1600, with most students scoring between 1000 and 1200.",
                ),
                (
                    "special education IEP accommodations",
                    "Special Education IEP Accommodations Guide",
                    "An Individualized Education Program outlines specific learning goals and accommodations for students with disabilities. Schools are required by IDEA to provide free appropriate public education.",
                ),
                (
                    "student loan repayment options",
                    "Federal Student Loan Repayment Options",
                    "Federal student loans offer income-driven repayment plans that cap monthly payments at a percentage of discretionary income. Public Service Loan Forgiveness is available after 120 qualifying payments.",
                ),
                (
                    "early childhood literacy development",
                    "Early Childhood Literacy Development Strategies",
                    "Early literacy development begins with phonological awareness and letter recognition. Reading aloud to children daily builds vocabulary, comprehension skills, and a positive association with books.",
                ),
            ],
            medium: &[
                (
                    "scalable digital instruction for global audiences",
                    "Asynchronous Learning Design for Massive Enrollment",
                    "Self-paced instructional modules with embedded assessments enable thousands of concurrent learners. Discussion forums and peer review create social learning opportunities at scale.",
                ),
                (
                    "measuring college readiness through exams",
                    "Psychometric Properties of High-Stakes Admissions Tests",
                    "Item response theory models estimate student ability from response patterns. Test equating procedures ensure score comparability across administrations despite different item sets.",
                ),
                (
                    "tailored support plans for learners with disabilities",
                    "Response to Intervention and Multi-Tiered Support Systems",
                    "Tiered frameworks provide increasingly intensive instruction based on progress monitoring data. Students who do not respond to general interventions are evaluated for specialized services.",
                ),
                (
                    "managing educational debt after graduation",
                    "Income-Contingent Repayment and Loan Servicing Complexities",
                    "Payment calculations adjust annually based on adjusted gross income and family size. Capitalized interest during forbearance periods can significantly increase total repayment cost.",
                ),
                (
                    "teaching young children to read and write",
                    "Phonemic Awareness Training and Decoding Fluency",
                    "Systematic phonics instruction explicitly teaches grapheme-phoneme correspondences. Decodable texts provide practice applying letter-sound knowledge to connected reading.",
                ),
            ],
            hard: &[
                (
                    "democratizing knowledge through technology at scale",
                    "Learning Analytics and Adaptive Pathways in Digital Pedagogy",
                    "Bayesian knowledge tracing models estimate mastery probability from interaction sequences. Adaptive algorithms select optimal next activities to maximize learning efficiency.",
                ),
                (
                    "statistical fairness in high-stakes academic evaluation",
                    "Differential Item Functioning and Measurement Invariance",
                    "Items exhibiting DIF perform differently across demographic subgroups after controlling for ability. Confirmatory factor analysis tests whether the construct structure is equivalent across populations.",
                ),
                (
                    "legal frameworks ensuring equitable access to learning",
                    "Endrew F. and the Substantive Standard for FAPE",
                    "The Supreme Court raised the bar from merely more than de minimis benefit to a program reasonably calculated to enable progress appropriate to the child's circumstances.",
                ),
                (
                    "long-term economic impact of educational borrowing",
                    "Human Capital Investment and Student Debt Drag on Wealth Accumulation",
                    "Net present value analysis of degree returns must account for opportunity cost, debt servicing burden, and the income-smoothing effect of progressive repayment. Racial wealth gaps amplify differential impacts.",
                ),
                (
                    "neural basis of reading acquisition in children",
                    "Orthographic Mapping and the Self-Teaching Hypothesis",
                    "Successful decoding attempts create bonded connections between spellings, pronunciations, and meanings in long-term memory. The ventral occipitotemporal cortex increasingly specializes for rapid word recognition.",
                ),
            ],
        },
        TopicTemplates {
            topic: "environment",
            easy: &[
                (
                    "carbon emissions climate change",
                    "Carbon Emissions and Climate Change Impact",
                    "Burning fossil fuels releases carbon dioxide into the atmosphere, trapping heat and raising global temperatures. Reducing carbon emissions is critical to limiting climate change.",
                ),
                (
                    "ocean plastic pollution",
                    "Ocean Plastic Pollution Crisis",
                    "Over 8 million tons of plastic enter the oceans annually. Marine animals ingest or become entangled in plastic debris, causing injury and death throughout the food chain.",
                ),
                (
                    "deforestation Amazon rainforest",
                    "Deforestation in the Amazon Rainforest",
                    "The Amazon rainforest loses thousands of square kilometers annually to logging, agriculture, and cattle ranching. Deforestation releases stored carbon and destroys biodiversity habitat.",
                ),
                (
                    "renewable energy solar panels",
                    "Solar Panel Renewable Energy Systems",
                    "Solar panels convert sunlight into electricity using photovoltaic cells. Residential solar installations can reduce electricity bills and carbon footprints significantly.",
                ),
                (
                    "endangered species conservation",
                    "Endangered Species Conservation Programs",
                    "Conservation programs protect endangered species through habitat preservation, captive breeding, anti-poaching enforcement, and wildlife corridors connecting fragmented ecosystems.",
                ),
            ],
            medium: &[
                (
                    "greenhouse gas atmospheric warming effects",
                    "Radiative Forcing and Global Energy Imbalance",
                    "Anthropogenic emissions alter the atmospheric radiative balance. Long-lived greenhouse gases trap outgoing infrared radiation, creating a persistent energy surplus that warms the climate system.",
                ),
                (
                    "synthetic debris accumulating in marine ecosystems",
                    "Microplastic Bioaccumulation in Pelagic Food Webs",
                    "Fragmented polymer particles are ingested by zooplankton and transferred up trophic levels. Persistent organic pollutants adsorb to plastic surfaces, concentrating toxins in predator tissues.",
                ),
                (
                    "tropical forest clearing for agriculture",
                    "Land Use Change and Carbon Flux in Tropical Biomes",
                    "Conversion of primary forest to pasture releases biomass carbon and reduces soil organic matter. Secondary regrowth sequesters carbon at diminishing rates compared to intact old-growth forest.",
                ),
                (
                    "converting sunlight to usable electricity",
                    "Photovoltaic Cell Physics and Grid Integration",
                    "Semiconductor junctions generate electron-hole pairs when absorbing photons above the band gap energy. Inverters convert DC output to AC, and net metering credits export to the grid.",
                ),
                (
                    "protecting at-risk wildlife populations",
                    "Minimum Viable Population Analysis for Conservation Planning",
                    "Population viability analysis models extinction probability under different management scenarios. Effective population size, genetic diversity, and habitat connectivity determine long-term persistence.",
                ),
            ],
            hard: &[
                (
                    "feedback loops accelerating planetary warming",
                    "Ice-Albedo Feedback and Permafrost Carbon Release Dynamics",
                    "Declining Arctic sea ice exposes dark ocean surface, increasing solar absorption. Thawing permafrost releases methane and CO2 from decomposing organic matter, amplifying the initial warming signal.",
                ),
                (
                    "persistent synthetic particles in ocean food chains",
                    "Trophic Transfer of Nanoplastics and Endocrine Disruption Mechanisms",
                    "Nanoscale polymer particles cross epithelial barriers and accumulate in organ tissues. Leached plasticizers and adsorbed xenoestrogens disrupt reproductive endocrinology in exposed organisms.",
                ),
                (
                    "ecological collapse from habitat fragmentation",
                    "Edge Effects and Metacommunity Dynamics in Forest Remnants",
                    "Increased perimeter-to-area ratios expose interior species to desiccation and invasive competitors. Stochastic local extinctions in isolated patches are not rescued by immigration, driving community simplification.",
                ),
                (
                    "semiconductor physics enabling clean power generation",
                    "Shockley-Queisser Limit and Tandem Cell Architectures",
                    "Single-junction efficiency is bounded by thermalization and transmission losses. Multijunction cells stack materials with different band gaps to harvest a broader spectrum, approaching 47% theoretical efficiency.",
                ),
                (
                    "genetic rescue of declining wild populations",
                    "Assisted Gene Flow and Outbreeding Depression Risk Assessment",
                    "Translocation of individuals from genetically divergent populations introduces novel alleles but risks disrupting co-adapted gene complexes. Genomic analysis guides selection of source populations to maximize adaptive potential.",
                ),
            ],
        },
        TopicTemplates {
            topic: "energy",
            easy: &[
                (
                    "wind turbine power generation",
                    "How Wind Turbines Generate Electricity",
                    "Wind turbines convert kinetic energy from wind into electrical energy. The rotating blades spin a generator, and modern turbines can produce several megawatts of power.",
                ),
                (
                    "nuclear fission power plants",
                    "Nuclear Fission Power Plant Operation",
                    "Nuclear fission splits heavy atoms like uranium-235 to release enormous energy. The heat produced creates steam that drives turbines to generate electricity.",
                ),
                (
                    "lithium ion battery storage",
                    "Lithium-Ion Battery Energy Storage",
                    "Lithium-ion batteries store energy by moving lithium ions between anode and cathode. They are used in electric vehicles, phones, and grid-scale energy storage systems.",
                ),
                (
                    "natural gas pipeline infrastructure",
                    "Natural Gas Pipeline Infrastructure",
                    "Natural gas is transported through extensive pipeline networks from production fields to consumers. Compressor stations maintain pressure along thousands of miles of transmission pipelines.",
                ),
                (
                    "geothermal energy heat pumps",
                    "Geothermal Energy and Heat Pump Systems",
                    "Geothermal energy harnesses heat from beneath the Earth's surface. Ground-source heat pumps use stable underground temperatures to efficiently heat and cool buildings.",
                ),
            ],
            medium: &[
                (
                    "harvesting kinetic energy from moving air",
                    "Aerodynamic Blade Design and Betz Limit",
                    "The theoretical maximum efficiency of a wind rotor is 59.3% of the kinetic energy in the airstream. Variable pitch mechanisms optimize power extraction across a range of wind speeds.",
                ),
                (
                    "splitting atoms to generate baseload electricity",
                    "Pressurized Water Reactor Thermodynamic Cycle",
                    "Enriched uranium fuel assemblies sustain a controlled chain reaction in the reactor core. A pressurized primary loop transfers thermal energy to a secondary steam cycle via heat exchangers.",
                ),
                (
                    "rechargeable electrochemical cells for grid storage",
                    "Intercalation Chemistry and Solid Electrolyte Interface Formation",
                    "Lithium ions shuttle between layered oxide cathodes and graphite anodes during charge and discharge. The SEI layer consumes electrolyte during initial cycles, determining long-term capacity retention.",
                ),
                (
                    "long-distance fossil fuel transport networks",
                    "Pipeline Hydraulics and Compressor Station Spacing",
                    "Flow equations balance pressure drop from friction against elevation changes and delivery requirements. Centrifugal compressors restore line pressure at intervals determined by throughput and terrain.",
                ),
                (
                    "tapping subsurface thermal gradients for power",
                    "Enhanced Geothermal Systems and Reservoir Stimulation",
                    "Hydraulic fracturing creates permeability in hot dry rock formations. Injected water circulates through the fracture network, absorbing heat and returning to the surface as steam.",
                ),
            ],
            hard: &[
                (
                    "maximizing extraction from atmospheric fluid dynamics",
                    "Wake Interaction Modeling in Offshore Wind Farm Layout Optimization",
                    "Downstream turbines experience reduced wind speed and increased turbulence from upstream wakes. Large-eddy simulations inform staggered placement strategies that maximize array energy production.",
                ),
                (
                    "next-generation reactor concepts beyond current designs",
                    "Molten Salt Reactor Neutronics and Online Reprocessing",
                    "Liquid fuel dissolved in fluoride salts circulates through the core, enabling continuous fission product removal. Negative temperature coefficients and low excess reactivity provide inherent safety without mechanical control rods.",
                ),
                (
                    "degradation mechanisms limiting cell longevity",
                    "Lithium Plating and Cathode Particle Cracking Under Fast Charge",
                    "High C-rates concentrate lithium flux at the anode surface, nucleating metallic deposits that consume cyclable lithium. Anisotropic volume changes fracture secondary cathode particles, exposing fresh surfaces to parasitic reactions.",
                ),
                (
                    "infrastructure resilience for fossil fuel distribution",
                    "Stress Corrosion Cracking and Integrity Management in Aging Pipelines",
                    "Cathodic protection and protective coatings mitigate external corrosion. In-line inspection tools detect wall thinning and crack-like defects, informing risk-based maintenance scheduling.",
                ),
                (
                    "accessing deep thermal resources in impermeable rock",
                    "Induced Seismicity Risk Assessment for Deep Geothermal Stimulation",
                    "Fluid injection increases pore pressure along pre-existing faults, potentially triggering felt seismic events. Traffic light protocols modulate injection parameters to keep induced magnitude below acceptable thresholds.",
                ),
            ],
        },
        TopicTemplates {
            topic: "agriculture",
            easy: &[
                (
                    "crop rotation soil health",
                    "Crop Rotation for Better Soil Health",
                    "Crop rotation alternates different crops in the same field across seasons. This practice reduces pest buildup, improves soil fertility, and prevents nutrient depletion.",
                ),
                (
                    "precision agriculture GPS technology",
                    "Precision Agriculture with GPS Technology",
                    "Precision agriculture uses GPS-guided equipment to apply fertilizers, pesticides, and water with centimeter-level accuracy, reducing waste and improving crop yields.",
                ),
                (
                    "organic farming pesticide free",
                    "Organic Farming Without Synthetic Pesticides",
                    "Organic farming avoids synthetic pesticides and fertilizers, relying on crop rotation, composting, biological pest control, and cover crops to maintain soil health and productivity.",
                ),
                (
                    "irrigation drip systems water conservation",
                    "Drip Irrigation for Water Conservation",
                    "Drip irrigation delivers water directly to plant roots through a network of tubes and emitters. It reduces water usage by 30-50% compared to traditional flood irrigation.",
                ),
                (
                    "greenhouse hydroponic farming",
                    "Hydroponic Farming in Greenhouses",
                    "Hydroponic systems grow plants in nutrient-rich water solutions without soil. Greenhouse hydroponics enables year-round production with precise control over growing conditions.",
                ),
            ],
            medium: &[
                (
                    "alternating plantings to maintain field productivity",
                    "Legume Integration and Biological Nitrogen Fixation",
                    "Rhizobium bacteria in legume root nodules convert atmospheric N2 to plant-available ammonium. Following cereals with legumes reduces synthetic fertilizer requirements for the subsequent crop.",
                ),
                (
                    "data-driven site-specific field management",
                    "Variable Rate Application and Yield Map Analytics",
                    "Georeferenced yield data reveals within-field productivity variation. Prescription maps adjust seeding rates and nutrient inputs to match spatial patterns of soil capacity.",
                ),
                (
                    "chemical-free crop production methods",
                    "Integrated Pest Management in Certified Organic Systems",
                    "Biological control agents, pheromone traps, and resistant cultivars suppress pest populations below economic thresholds. Mechanical cultivation and flame weeding replace herbicide applications.",
                ),
                (
                    "efficient targeted water delivery to crops",
                    "Soil Moisture Sensor Networks and Irrigation Scheduling",
                    "Capacitance probes at multiple depths track the wetting front through the root zone. Evapotranspiration models calibrated to sensor data trigger irrigation when plant-available water drops below a threshold.",
                ),
                (
                    "soil-free plant cultivation in controlled environments",
                    "Nutrient Film Technique and pH-EC Management",
                    "A thin stream of recirculating solution bathes exposed roots, supplying macro- and micronutrients. Automated dosing maintains target pH and electrical conductivity for optimal nutrient uptake.",
                ),
            ],
            hard: &[
                (
                    "microbial communities driving nutrient cycling in soils",
                    "Mycorrhizal Networks and Phosphorus Acquisition in Agricultural Systems",
                    "Arbuscular mycorrhizal fungi extend hyphal networks beyond the root depletion zone, accessing phosphorus locked in soil mineral surfaces. Carbon allocation to fungal symbionts trades photosynthate for mineral nutrients.",
                ),
                (
                    "satellite-guided autonomous field operations",
                    "Multi-Spectral Remote Sensing and Crop Stress Detection Algorithms",
                    "Normalized difference vegetation index anomalies identify zones of biotic or abiotic stress. Machine learning classifiers trained on ground-truth data distinguish nitrogen deficiency from water stress using spectral signatures.",
                ),
                (
                    "ecosystem services from diversified farming systems",
                    "Agroecological Intensification and Functional Biodiversity",
                    "Polyculture systems leverage complementary resource use to exceed monoculture yields. Floral margins and beetle banks enhance natural enemy populations, providing biological pest regulation across the landscape.",
                ),
                (
                    "optimizing water application to match crop physiology",
                    "Deficit Irrigation Strategies and Regulated Deficit Timing",
                    "Controlled water stress during non-critical growth stages diverts assimilates to reproductive organs. Yield quality may improve even as total biomass production declines under carefully managed deficit.",
                ),
                (
                    "methane emissions from enteric fermentation in ruminants",
                    "Methanogenesis Inhibition Through Dietary 3-Nitrooxypropanol Supplementation",
                    "The synthetic compound blocks the final step of methanogenesis in the rumen, reducing enteric methane by up to 30% without affecting feed intake or milk production.",
                ),
            ],
        },
        TopicTemplates {
            topic: "transportation",
            easy: &[
                (
                    "electric vehicle battery range",
                    "Electric Vehicle Battery Range Explained",
                    "Electric vehicle range depends on battery capacity, driving conditions, and climate. Modern EVs typically offer 200-400 miles of range on a full charge.",
                ),
                (
                    "autonomous self-driving cars",
                    "Autonomous Self-Driving Car Technology",
                    "Self-driving cars use lidar, cameras, radar, and AI to navigate without human input. SAE defines six levels of automation from Level 0 to Level 5.",
                ),
                (
                    "high speed rail systems",
                    "High-Speed Rail Transportation Systems",
                    "High-speed rail operates at speeds above 200 km/h on dedicated tracks. Japan's Shinkansen, France's TGV, and China's CRH network demonstrate the technology's potential.",
                ),
                (
                    "urban bicycle infrastructure",
                    "Urban Bicycle Infrastructure Planning",
                    "Protected bike lanes, bike-sharing programs, and secure parking facilities encourage cycling as urban transportation. Cities with good cycling infrastructure see reduced traffic congestion and emissions.",
                ),
                (
                    "commercial aviation fuel efficiency",
                    "Commercial Aviation Fuel Efficiency Improvements",
                    "Modern commercial aircraft are 80% more fuel-efficient per passenger-mile than jets from the 1960s. Winglets, lighter composite materials, and more efficient engines drive ongoing improvements.",
                ),
            ],
            medium: &[
                (
                    "factors limiting distance per charge in EVs",
                    "Battery Thermal Management and Range Optimization",
                    "Lithium-ion cell performance degrades at temperature extremes. Active cooling and heating systems maintain optimal pack temperature, balancing energy consumption against range preservation.",
                ),
                (
                    "computer-controlled vehicles without human drivers",
                    "Sensor Fusion Architectures for Autonomous Navigation",
                    "Complementary sensor modalities compensate for individual weaknesses. Lidar provides precise depth measurement while cameras supply color and texture information for semantic understanding.",
                ),
                (
                    "very fast trains on dedicated corridors",
                    "Cant Deficiency and Curve Speed Limits on High-Speed Track",
                    "Superelevation tilts the track to counteract centrifugal force in curves. Tilting train mechanisms allow higher speeds by compensating for the remaining lateral acceleration experienced by passengers.",
                ),
                (
                    "designing cities for human-powered mobility",
                    "Modal Shift Strategies and Last-Mile Connectivity",
                    "Integrating cycling and walking networks with public transit hubs reduces car dependency. Traffic calming measures and low-emission zones prioritize active mobility in urban centers.",
                ),
                (
                    "reducing fuel burn in passenger aircraft",
                    "Laminar Flow Control and Drag Reduction on Airframes",
                    "Natural and hybrid laminar flow surfaces delay boundary layer transition, reducing skin friction drag. Active suction through porous panels maintains favorable pressure gradients on swept wings.",
                ),
            ],
            hard: &[
                (
                    "electrochemical degradation limiting vehicle longevity",
                    "Cathode-Electrolyte Interphase Evolution in Automotive Battery Packs",
                    "Transition metal dissolution from cathode active materials deposits on the anode, catalyzing electrolyte decomposition. Calendar aging models predict capacity fade as a function of temperature and state of charge.",
                ),
                (
                    "ethical decision-making in driverless vehicle software",
                    "Trolley Problem Formalization in Autonomous Vehicle Risk Allocation",
                    "Utilitarian frameworks conflict with deontological constraints when unavoidable collisions require choosing between harm recipients. Regulatory approaches increasingly favor rule-based systems over outcome-optimization.",
                ),
                (
                    "civil engineering for ultra-high-speed ground transport",
                    "Slab Track Design and Settlement Tolerance for 350 km/h Operation",
                    "Continuous reinforced concrete slab track eliminates ballast maintenance requirements. Sub-millimeter settlement tolerances demand ground improvement techniques and real-time structural health monitoring.",
                ),
                (
                    "reallocation of road space from cars to people",
                    "Induced Demand Reversal and Traffic Evaporation in Car-Free Zones",
                    "Removing road capacity does not proportionally increase congestion on alternative routes. Trip suppression, mode shift, and route redistribution absorb latent demand, a phenomenon termed traffic evaporation.",
                ),
                (
                    "aerodynamic optimization of next-generation airframes",
                    "Blended Wing Body Configurations and Structural-Aerodynamic Coupling",
                    "Flying wing designs offer substantial wetted area reduction and improved lift-to-drag ratios. Non-circular pressurized cabins require novel structural solutions to handle hoop stress efficiently.",
                ),
            ],
        },
        TopicTemplates {
            topic: "manufacturing",
            easy: &[
                (
                    "3D printing additive manufacturing",
                    "3D Printing and Additive Manufacturing",
                    "3D printing builds objects layer by layer from digital models. Technologies include FDM, SLA, and SLS, each suited to different materials and applications.",
                ),
                (
                    "lean manufacturing waste reduction",
                    "Lean Manufacturing and Waste Reduction",
                    "Lean manufacturing identifies and eliminates eight types of waste: defects, overproduction, waiting, non-utilized talent, transportation, inventory, motion, and extra processing.",
                ),
                (
                    "industrial robot assembly line",
                    "Industrial Robots on Assembly Lines",
                    "Industrial robots perform welding, painting, assembly, and inspection tasks with high precision and speed. They operate 24/7 and maintain consistent quality across thousands of repetitions.",
                ),
                (
                    "quality control inspection processes",
                    "Manufacturing Quality Control Inspection",
                    "Quality control uses statistical sampling, visual inspection, and automated testing to detect defects. Six Sigma methodology aims for fewer than 3.4 defects per million opportunities.",
                ),
                (
                    "CNC machining precision parts",
                    "CNC Machining for Precision Parts",
                    "CNC machines use computer-controlled tools to cut, drill, and shape materials with micrometer precision. G-code programs direct the cutting path for complex geometries.",
                ),
            ],
            medium: &[
                (
                    "building objects layer by layer from digital files",
                    "Powder Bed Fusion and Part Density Optimization",
                    "Laser parameters including power, speed, and hatch spacing determine melt pool geometry. Process maps identify parameter windows that achieve full density while minimizing residual stress.",
                ),
                (
                    "eliminating inefficiency in production systems",
                    "Value Stream Mapping and Continuous Flow Implementation",
                    "Mapping material and information flows reveals non-value-adding steps. Takt time calculations synchronize production pace with customer demand, eliminating overproduction.",
                ),
                (
                    "programmable machines performing repetitive factory tasks",
                    "Collaborative Robot Safety Standards and Force Limiting",
                    "ISO/TS 15066 defines allowable contact forces for human-robot collaboration. Power and force limiting mode enables robots to work alongside humans without physical barriers.",
                ),
                (
                    "systematic defect prevention in production",
                    "Statistical Process Control and Control Chart Interpretation",
                    "X-bar and R charts monitor process mean and variability in real time. Out-of-control patterns trigger root cause investigation before defective product reaches the customer.",
                ),
                (
                    "computer-guided material removal for complex shapes",
                    "Tool Path Generation and Adaptive Machining Strategies",
                    "CAM software calculates cutter engagement along complex surfaces. Adaptive algorithms adjust feed rates based on material removal rate, preventing tool overload and chatter.",
                ),
            ],
            hard: &[
                (
                    "metallurgical properties of additively manufactured parts",
                    "Epitaxial Grain Growth and Anisotropy in Directed Energy Deposition",
                    "Columnar grains grow preferentially along the build direction following the thermal gradient. Resulting anisotropic mechanical properties require post-process hot isostatic pressing to close internal porosity and recrystallize.",
                ),
                (
                    "organizational transformation toward operational excellence",
                    "Theory of Constraints Integration with Lean and Six Sigma",
                    "Identifying and exploiting the system bottleneck maximizes throughput. Drum-buffer-rope scheduling subordinates all non-constraint resources to the constraint pace, while Six Sigma reduces variability at the bottleneck.",
                ),
                (
                    "human-machine cooperative manufacturing systems",
                    "Digital Twin Synchronization for Adaptive Robotic Assembly",
                    "Real-time kinematic models mirror physical robot states. Deviation detection triggers automatic trajectory replanning, enabling the system to adapt to part variability without manual reprogramming.",
                ),
                (
                    "detecting subtle process drift before defects occur",
                    "Multivariate Statistical Process Monitoring with Principal Component Analysis",
                    "Hotelling T-squared and Q-statistics detect correlated shifts across multiple quality characteristics simultaneously. Contribution plots identify which variables drive the alarm for targeted corrective action.",
                ),
                (
                    "physics-based simulation of chip formation in cutting",
                    "Finite Element Modeling of Thermomechanical Coupling in Orthogonal Machining",
                    "Johnson-Cook constitutive models capture strain-rate and temperature dependent material flow. Coupled thermal-mechanical simulations predict tool wear, chip morphology, and residual stress in the machined surface.",
                ),
            ],
        },
        TopicTemplates {
            topic: "telecommunications",
            easy: &[
                (
                    "5G network speed coverage",
                    "5G Network Speed and Coverage",
                    "5G networks offer speeds up to 10 Gbps with latency under 1 millisecond. Coverage uses three spectrum bands: low-band for range, mid-band for balance, and mmWave for speed.",
                ),
                (
                    "fiber optic internet connection",
                    "Fiber Optic Internet Connections",
                    "Fiber optic cables transmit data as light pulses through glass strands. They offer gigabit speeds with lower latency and higher reliability than copper-based connections.",
                ),
                (
                    "satellite internet rural areas",
                    "Satellite Internet for Rural Areas",
                    "Low-earth orbit satellite constellations like Starlink provide broadband internet to remote areas. LEO satellites orbit at 550 km, offering lower latency than traditional geostationary satellites.",
                ),
                (
                    "VoIP phone service technology",
                    "VoIP Phone Service Technology",
                    "Voice over IP converts voice signals into digital data packets transmitted over the internet. VoIP offers lower costs and more features than traditional phone lines.",
                ),
                (
                    "cellular tower signal propagation",
                    "Cellular Tower Signal Propagation",
                    "Cell towers transmit radio signals in specific frequency bands. Signal strength decreases with distance and is affected by buildings, terrain, and atmospheric conditions.",
                ),
            ],
            medium: &[
                (
                    "next-generation mobile broadband capabilities",
                    "Massive MIMO and Beamforming in Sub-6 GHz Deployments",
                    "Antenna arrays with 64 or more elements form focused beams toward individual users. Spatial multiplexing serves multiple devices simultaneously on the same time-frequency resource.",
                ),
                (
                    "light-based data transmission through glass strands",
                    "Wavelength Division Multiplexing and Optical Amplification",
                    "Multiple independent channels occupy different wavelengths on a single fiber. Erbium-doped fiber amplifiers boost all channels simultaneously without optical-electrical conversion.",
                ),
                (
                    "orbital broadband for underserved communities",
                    "Inter-Satellite Laser Links and Constellation Routing",
                    "Optical crosslinks between satellites create a mesh network in space. Shortest-path algorithms route traffic through the constellation, reducing reliance on terrestrial ground stations.",
                ),
                (
                    "real-time voice communication over packet networks",
                    "Jitter Buffer Management and Codec Selection for Low-Latency Voice",
                    "Adaptive jitter buffers smooth packet arrival variation while minimizing end-to-end delay. Opus and G.722 codecs provide wideband audio quality at bit rates compatible with constrained links.",
                ),
                (
                    "radio wave behavior between base stations and devices",
                    "Path Loss Models and Small Cell Densification Strategies",
                    "Log-distance path loss models predict signal attenuation as a function of frequency and environment. Dense small cell deployment offloads macro cell traffic, increasing area capacity.",
                ),
            ],
            hard: &[
                (
                    "spectral efficiency limits of wireless channels",
                    "Shannon Capacity and Non-Orthogonal Multiple Access Schemes",
                    "Superposition coding at the transmitter and successive interference cancellation at the receiver approach capacity bounds. Power-domain NOMA assigns different power levels to multiplexed users on the same resource.",
                ),
                (
                    "quantum effects limiting fiber transmission reach",
                    "Kerr Nonlinearity and the Nonlinear Shannon Limit in Optical Fiber",
                    "Signal-signal four-wave mixing generates in-band interference at high launch powers. Digital back-propagation algorithms partially compensate nonlinear distortion but increase transceiver computational load.",
                ),
                (
                    "orbital mechanics constraints on communication networks",
                    "Doppler Compensation and Handover Optimization in LEO Mega-Constellations",
                    "Relative velocity between LEO satellites and ground terminals creates frequency shifts requiring real-time correction. Walker constellation geometries balance coverage uniformity against inter-plane handover frequency.",
                ),
                (
                    "perceptual quality optimization in compressed speech",
                    "Wideband Audio Coding and Perceptual Entropy Estimation",
                    "Modified discrete cosine transform coefficients are quantized according to psychoacoustic masking thresholds. Rate-distortion optimization selects quantizer step sizes that maximize perceptual quality at target bitrate.",
                ),
                (
                    "electromagnetic propagation in complex urban environments",
                    "Ray Tracing Simulation and Diffraction Modeling for mmWave Coverage",
                    "Shooting-and-bouncing-ray algorithms trace multipath propagation through 3D building databases. Uniform theory of diffraction corrections account for energy scattered around corners and rooftop edges.",
                ),
            ],
        },
        TopicTemplates {
            topic: "entertainment",
            easy: &[
                (
                    "streaming service subscription comparison",
                    "Streaming Service Subscription Comparison",
                    "Netflix, Disney+, HBO Max, and Amazon Prime Video offer different content libraries and pricing tiers. Most services now offer ad-supported plans at lower monthly costs.",
                ),
                (
                    "video game graphics rendering",
                    "Video Game Graphics Rendering Technology",
                    "Real-time rendering uses rasterization and ray tracing to create game visuals. GPUs process millions of polygons per frame at 60 or more frames per second.",
                ),
                (
                    "music production digital audio workstation",
                    "Music Production with Digital Audio Workstations",
                    "Digital audio workstations like Ableton, Logic Pro, and FL Studio enable music creation with virtual instruments, audio recording, mixing, and mastering capabilities.",
                ),
                (
                    "film special effects CGI",
                    "Film Special Effects and CGI Technology",
                    "Computer-generated imagery creates realistic visual effects in movies. Motion capture, particle simulations, and physics-based rendering produce photorealistic characters and environments.",
                ),
                (
                    "virtual reality gaming headsets",
                    "Virtual Reality Gaming Headsets",
                    "VR headsets like Meta Quest and PlayStation VR2 provide immersive gaming through stereoscopic displays, head tracking, and hand controllers with haptic feedback.",
                ),
            ],
            medium: &[
                (
                    "comparing digital content delivery platforms",
                    "Content Recommendation Algorithms and Viewer Retention",
                    "Collaborative filtering and deep learning models predict user preferences from viewing history. Personalized thumbnails and autoplay sequencing optimize engagement metrics and reduce churn.",
                ),
                (
                    "real-time image synthesis for interactive media",
                    "Deferred Rendering Pipelines and Global Illumination Approximations",
                    "Geometry buffer passes store surface attributes for lighting calculations. Screen-space reflections and ambient occlusion approximate global illumination at interactive frame rates.",
                ),
                (
                    "computer-based sound design and composition",
                    "MIDI Sequencing and Virtual Synthesizer Architecture",
                    "Software synthesizers model analog signal chains through digital signal processing. Wavetable, subtractive, and FM synthesis techniques generate diverse timbres from mathematical oscillator models.",
                ),
                (
                    "creating realistic digital characters for cinema",
                    "Subsurface Scattering and Physically-Based Skin Rendering",
                    "Light penetrates translucent skin tissue and scatters before exiting. Dipole diffusion models simulate this transport, producing the soft, warm appearance of realistic human skin.",
                ),
                (
                    "immersive head-mounted display experiences",
                    "Vestibular-Ocular Conflict and Motion Sickness Mitigation in VR",
                    "Discrepancy between visual motion cues and vestibular input causes simulator sickness. Fixed reference frames, reduced vection, and high refresh rates minimize sensory conflict.",
                ),
            ],
            hard: &[
                (
                    "algorithmic curation shaping cultural consumption patterns",
                    "Filter Bubbles and Epistemic Diversity in Recommender Systems",
                    "Optimizing for engagement reinforces existing preferences, narrowing content exposure. Exploration-exploitation trade-offs in bandit algorithms can be tuned to promote serendipitous discovery.",
                ),
                (
                    "physically accurate light simulation in real time",
                    "Path Tracing Convergence and Spatiotemporal Denoising in Real-Time Rendering",
                    "Monte Carlo path tracing at one sample per pixel produces high-variance images. Neural network denoisers trained on converged references reconstruct clean frames from noisy input.",
                ),
                (
                    "psychoacoustic principles in digital sound creation",
                    "Spectral Masking and Loudness Normalization in Mastering Workflows",
                    "Simultaneous masking thresholds determine perceptible frequency content. LUFS-based loudness targeting ensures consistent playback levels across distribution platforms without dynamic range compression artifacts.",
                ),
                (
                    "digital resurrection of actors through synthetic media",
                    "Neural Radiance Fields for Digital Human Reconstruction",
                    "NeRF models represent a person as a continuous volumetric function learned from multi-view images. Relightable variants decompose appearance into reflectance and illumination for compositing into novel scenes.",
                ),
                (
                    "neurological effects of prolonged virtual immersion",
                    "Cybersickness Biomarkers and Electrodermal Activity Monitoring",
                    "Galvanic skin response correlates with simulator sickness onset before subjective awareness. Closed-loop systems that modulate rendering parameters based on physiological signals can preemptively reduce discomfort.",
                ),
            ],
        },
        TopicTemplates {
            topic: "sports",
            easy: &[
                (
                    "marathon training schedule beginners",
                    "Marathon Training Schedule for Beginners",
                    "A beginner marathon training plan spans 16-20 weeks with gradual mileage increases. Key elements include long runs, rest days, cross-training, and tapering before race day.",
                ),
                (
                    "NBA basketball salary cap",
                    "NBA Basketball Salary Cap Rules",
                    "The NBA salary cap limits team spending on player salaries. Luxury tax penalties apply to teams exceeding the cap, while exceptions allow re-signing certain players above it.",
                ),
                (
                    "sports analytics player statistics",
                    "Sports Analytics and Player Statistics",
                    "Sports analytics uses statistical models to evaluate player performance and strategy. Metrics like WAR in baseball and PER in basketball quantify individual contributions beyond traditional stats.",
                ),
                (
                    "football concussion protocol",
                    "NFL Football Concussion Protocol",
                    "The NFL concussion protocol requires players showing symptoms to be evaluated by team physicians and independent neurological consultants before returning to play.",
                ),
                (
                    "swimming freestyle technique tips",
                    "Freestyle Swimming Technique Tips",
                    "Proper freestyle technique includes high elbow catch, body rotation, bilateral breathing, and a steady six-beat kick. Streamlined body position reduces drag and improves efficiency.",
                ),
            ],
            medium: &[
                (
                    "structured endurance preparation for long distance running",
                    "Periodization and Heart Rate Zone Training for Distance Runners",
                    "Base building phases develop aerobic capacity through high-volume low-intensity work. Threshold and interval sessions improve lactate clearance and VO2 max during the specific preparation block.",
                ),
                (
                    "economic constraints on professional team roster construction",
                    "Collective Bargaining Agreements and Revenue Sharing in Pro Sports",
                    "Salary floors and ceilings redistribute league revenue between large and small market teams. Bird rights and mid-level exceptions create flexibility for roster construction within cap constraints.",
                ),
                (
                    "data-driven performance evaluation in athletics",
                    "Expected Goals Models and Spatial Event Data in Football Analytics",
                    "Logistic regression models estimate the probability of scoring from each shot based on location, body part, and defensive pressure. Cumulative xG metrics evaluate attacking and defensive quality.",
                ),
                (
                    "protecting athletes from brain injuries",
                    "Return-to-Play Protocols and Chronic Traumatic Encephalopathy Risk",
                    "Graduated return-to-play protocols progress through symptom-free stages. Accumulation of subconcussive impacts may contribute to neurodegenerative pathology independent of diagnosed concussions.",
                ),
                (
                    "optimizing body position in competitive swimming",
                    "Drag Coefficient Reduction Through Stroke Kinematics Analysis",
                    "Underwater motion capture systems quantify joint angles and hand velocity profiles. Computational fluid dynamics simulations identify stroke modifications that reduce form drag and increase propulsive efficiency.",
                ),
            ],
            hard: &[
                (
                    "physiological adaptation to extreme endurance training",
                    "Mitochondrial Biogenesis and Fat Oxidation Kinetics in Ultra-Endurance Athletes",
                    "PGC-1alpha-mediated transcription upregulates mitochondrial enzyme complexes in slow-twitch fibers. Enhanced fat oxidation rates spare glycogen reserves, extending time to exhaustion at submaximal intensities.",
                ),
                (
                    "game theory in professional league salary negotiations",
                    "Nash Bargaining Solutions and Surplus Division in Player Contract Negotiations",
                    "The contract zone is bounded by each party's outside option. Asymmetric information about injury risk and performance trajectory creates strategic incentives for signaling and screening.",
                ),
                (
                    "causal inference in athletic performance measurement",
                    "Counterfactual Frameworks for Isolating Player Contribution in Team Sports",
                    "Regression-adjusted plus-minus estimates individual impact by controlling for teammate and opponent quality. Bayesian hierarchical models share information across partial pooling groups to stabilize estimates for players with limited minutes.",
                ),
                (
                    "biomechanical forces in repeated head impacts",
                    "Finite Element Head Models and Rotational Acceleration Thresholds",
                    "Computational head models simulate brain tissue deformation under measured impact kinematics. Cumulative strain metrics in vulnerable white matter regions correlate with diffusion tensor imaging abnormalities.",
                ),
                (
                    "fluid dynamics of competitive aquatic locomotion",
                    "Vortex Shedding and Wake Momentum Analysis in Elite Swimming",
                    "Particle image velocimetry reveals vortex ring structures generated during the hand pull. Wake momentum deficit calculations quantify propulsive force production throughout the stroke cycle.",
                ),
            ],
        },
        TopicTemplates {
            topic: "politics",
            easy: &[
                (
                    "electoral college presidential election",
                    "The Electoral College in Presidential Elections",
                    "The Electoral College determines the US president through 538 electors allocated by state. A candidate needs 270 electoral votes to win. Most states use winner-take-all allocation.",
                ),
                (
                    "congressional filibuster rules",
                    "Senate Filibuster Rules Explained",
                    "A Senate filibuster allows unlimited debate to delay or block legislation. Ending a filibuster requires a cloture vote with 60 senators, creating a supermajority threshold for passing bills.",
                ),
                (
                    "campaign finance regulations",
                    "Campaign Finance Regulations and Limits",
                    "Federal election law limits individual donations to candidates and parties. Super PACs can raise unlimited funds for independent expenditures but cannot coordinate directly with campaigns.",
                ),
                (
                    "gerrymandering district boundaries",
                    "Gerrymandering and District Boundary Drawing",
                    "Gerrymandering manipulates electoral district boundaries to favor one party. Techniques include packing opponents into few districts and cracking their voters across many districts.",
                ),
                (
                    "voter registration requirements",
                    "Voter Registration Requirements by State",
                    "Voter registration requirements vary by state and may include ID verification, proof of residence, and registration deadlines. Some states offer same-day registration at polling places.",
                ),
            ],
            medium: &[
                (
                    "indirect election mechanism for head of state",
                    "Federalist Arguments for Mediated Presidential Selection",
                    "The constitutional framers designed the Electoral College as a deliberative body insulating executive selection from direct popular passion. State-based allocation reflects the federal bargain between large and small states.",
                ),
                (
                    "supermajority requirements blocking legislative action",
                    "Cloture Evolution and the Rise of the Silent Filibuster",
                    "Modern filibusters rarely require floor speeches. The dual-track system allows other business to proceed while a filibustered measure remains in limbo, reducing the political cost of obstruction.",
                ),
                (
                    "money in electoral politics and democratic integrity",
                    "Buckley-Valeo Doctrine and the Speech-Money Nexus",
                    "The Supreme Court equated campaign spending with protected speech. Contribution limits survived scrutiny as anti-corruption measures, but independent expenditure caps were struck down as unconstitutional.",
                ),
                (
                    "manipulating electoral geography for partisan advantage",
                    "Efficiency Gap Metric and Partisan Symmetry Standards",
                    "The efficiency gap measures wasted votes across districts to quantify partisan asymmetry. Courts have considered mathematical standards but struggled to define a judicially manageable threshold for unconstitutional gerrymandering.",
                ),
                (
                    "barriers to electoral participation across jurisdictions",
                    "Strict ID Laws and Differential Turnout Effects by Demographic",
                    "Studies compare turnout changes in states adopting strict photo ID requirements. Disparate impact on minority and low-income voters raises equal protection concerns under the Fourteenth Amendment.",
                ),
            ],
            hard: &[
                (
                    "strategic voting behavior in multi-candidate contests",
                    "Duverger's Law and Strategic Entry Deterrence in Plurality Systems",
                    "First-past-the-post elections tend toward two-party equilibria as voters abandon non-viable candidates. Ranked-choice and proportional systems alter strategic calculus, enabling multi-party representation.",
                ),
                (
                    "procedural minority power in deliberative assemblies",
                    "Pivotal Politics Theory and the Filibuster-Veto Gridlock Interval",
                    "Krehbiel's model identifies the gridlock interval between the veto pivot and filibuster pivot. Policy change requires shocks sufficient to move the status quo outside this interval.",
                ),
                (
                    "institutional design for regulating democratic competition",
                    "Public Financing Models and Small-Dollar Matching Fund Amplification",
                    "Matching fund programs multiply small contributions, incentivizing broad-based fundraising. Empirical analysis of New York City's program shows increased donor diversity and candidate competitiveness.",
                ),
                (
                    "computational approaches to fair redistricting",
                    "Markov Chain Monte Carlo Ensemble Analysis for Redistricting Evaluation",
                    "Random walk algorithms generate large ensembles of legally valid district plans. Comparing an enacted plan's partisan metrics to the ensemble distribution quantifies the probability that observed asymmetry arose from neutral criteria.",
                ),
                (
                    "franchise expansion and contraction throughout history",
                    "Shelby County and the Dismantling of Federal Preclearance Requirements",
                    "The Supreme Court invalidated the coverage formula of Section 4(b) of the Voting Rights Act. Absent preclearance, jurisdictions with histories of discrimination adopted restrictive measures previously blocked by DOJ review.",
                ),
            ],
        },
        TopicTemplates {
            topic: "real_estate",
            easy: &[
                (
                    "home buying closing costs",
                    "Understanding Home Buying Closing Costs",
                    "Closing costs typically range from 2-5% of the home purchase price and include loan origination fees, title insurance, appraisal fees, escrow deposits, and recording fees.",
                ),
                (
                    "rental property investment returns",
                    "Rental Property Investment Return Analysis",
                    "Rental property returns depend on cash flow, appreciation, tax benefits, and equity buildup. Cap rate and cash-on-cash return are key metrics for evaluating investment properties.",
                ),
                (
                    "real estate market housing prices",
                    "Housing Market Price Trends and Analysis",
                    "Housing prices are influenced by interest rates, inventory levels, employment, and demographic trends. The Case-Shiller index tracks residential real estate prices across major metropolitan areas.",
                ),
                (
                    "commercial real estate lease types",
                    "Types of Commercial Real Estate Leases",
                    "Commercial leases include gross, net, and modified gross structures. Triple net leases require tenants to pay property taxes, insurance, and maintenance in addition to base rent.",
                ),
                (
                    "property zoning regulations",
                    "Property Zoning Regulations and Land Use",
                    "Zoning regulations divide land into residential, commercial, industrial, and mixed-use zones. Variances and special use permits allow exceptions to standard zoning restrictions.",
                ),
            ],
            medium: &[
                (
                    "transaction expenses when purchasing a home",
                    "Title Search and Escrow Procedures in Residential Closings",
                    "Title companies examine public records to verify ownership chain and identify liens. Escrow agents hold funds and documents, disbursing them when all closing conditions are satisfied.",
                ),
                (
                    "income-generating property financial analysis",
                    "Discounted Cash Flow Modeling for Multifamily Acquisitions",
                    "Pro forma projections model rent growth, vacancy, and operating expense trends over a holding period. Internal rate of return calculations incorporate leverage, capital expenditure, and disposition proceeds.",
                ),
                (
                    "factors driving residential property values",
                    "Supply Elasticity and Housing Affordability in Constrained Markets",
                    "Geographic and regulatory constraints on new construction limit housing supply response to demand increases. Price-to-income ratios diverge in markets where permitting delays and zoning restrictions restrict density.",
                ),
                (
                    "business property rental agreements and structures",
                    "Build-to-Suit Leases and Tenant Improvement Allowance Negotiations",
                    "Landlords amortize tenant improvement costs into higher base rent over the lease term. Creditworthy tenants negotiate larger allowances by offering longer terms and stronger guarantees.",
                ),
                (
                    "government control of land development patterns",
                    "Form-Based Codes and Transit-Oriented Development Incentives",
                    "Form-based codes regulate building form rather than use, enabling mixed-use development by right. Density bonuses near transit stations incentivize housing production in accessible locations.",
                ),
            ],
            hard: &[
                (
                    "risk stratification in mortgage-backed instruments",
                    "Subordination Structures and Credit Enhancement in CMBS Tranching",
                    "Sequential pay waterfalls direct losses to junior tranches first, protecting senior bondholders. Debt service coverage ratio and loan-to-value triggers redirect cash flow when pool performance deteriorates.",
                ),
                (
                    "tax-advantaged structures for property investment",
                    "1031 Exchange Qualified Intermediary Requirements and Boot Recognition",
                    "Like-kind exchanges defer capital gains tax by reinvesting proceeds into replacement property within 180 days. Boot, that is cash or mortgage relief not reinvested, triggers partial gain recognition.",
                ),
                (
                    "macroprudential factors in housing cycle dynamics",
                    "Credit Channel Amplification and Housing Wealth Effects on Consumption",
                    "Rising home values increase collateral values, relaxing borrowing constraints and amplifying credit expansion. The feedback loop between asset prices and credit availability contributed to pre-2008 housing bubble dynamics.",
                ),
                (
                    "optimal rent extraction under information asymmetry",
                    "Hedonic Pricing Models and Lease Renewal Retention Analysis",
                    "Hedonic regression isolates the marginal value of individual property attributes. Renewal probability models balance rent escalation against tenant turnover costs and vacancy exposure.",
                ),
                (
                    "legal constraints on urban density and development form",
                    "Euclidean Zoning Challenges and the Missing Middle Housing Debate",
                    "Single-family zoning exclusions restrict naturally affordable housing typologies like duplexes and fourplexes. Reform efforts face political opposition from incumbent homeowners benefiting from artificial scarcity.",
                ),
            ],
        },
        TopicTemplates {
            topic: "cybersecurity",
            easy: &[
                (
                    "ransomware attack prevention",
                    "Preventing Ransomware Attacks",
                    "Ransomware encrypts files and demands payment for decryption. Prevention includes regular backups, software updates, email filtering, network segmentation, and employee security awareness training.",
                ),
                (
                    "two factor authentication security",
                    "Two-Factor Authentication Security",
                    "Two-factor authentication requires something you know (password) and something you have (phone or hardware key). It significantly reduces the risk of unauthorized account access.",
                ),
                (
                    "SQL injection attack prevention",
                    "Preventing SQL Injection Attacks",
                    "SQL injection exploits vulnerable database queries by inserting malicious SQL code through user input fields. Parameterized queries and input validation prevent this common web vulnerability.",
                ),
                (
                    "phishing email detection",
                    "How to Detect Phishing Emails",
                    "Phishing emails impersonate legitimate senders to steal credentials or install malware. Warning signs include urgent language, suspicious URLs, misspelled domains, and unexpected attachments.",
                ),
                (
                    "firewall network security",
                    "Firewall Network Security Configuration",
                    "Firewalls filter network traffic based on predefined rules. Next-generation firewalls add application-level inspection, intrusion prevention, and threat intelligence to traditional packet filtering.",
                ),
            ],
            medium: &[
                (
                    "defending against file-encrypting extortion malware",
                    "Endpoint Detection and Response for Crypto-Ransomware Containment",
                    "Behavioral analysis detects mass file encryption patterns and kills the offending process. Automated network isolation prevents lateral movement while preserving forensic artifacts for investigation.",
                ),
                (
                    "adding a second verification step beyond passwords",
                    "FIDO2 WebAuthn and Phishing-Resistant Authentication Ceremonies",
                    "Public key credentials bound to the relying party origin prevent phishing by design. Hardware authenticators generate unique key pairs per service, eliminating shared secrets from the authentication flow.",
                ),
                (
                    "exploiting unsanitized input in database queries",
                    "Object-Relational Mapping and Prepared Statement Compilation",
                    "ORM frameworks abstract SQL generation, parameterizing user input by default. Stored procedures with typed parameters provide an additional defense layer against injection in legacy codebases.",
                ),
                (
                    "social engineering through fraudulent communications",
                    "Business Email Compromise and Domain Spoofing Countermeasures",
                    "DMARC policies instruct receiving servers to reject messages failing SPF and DKIM alignment. AI-based anomaly detection flags deviations in communication patterns that indicate compromised accounts.",
                ),
                (
                    "traffic filtering at network boundaries",
                    "Zero Trust Architecture and Microsegmentation Beyond Perimeter Defense",
                    "Identity-based access policies replace implicit trust from network location. Microsegmentation limits lateral movement by enforcing least-privilege communication between workloads.",
                ),
            ],
            hard: &[
                (
                    "post-compromise persistence and privilege escalation",
                    "Living-Off-the-Land Binaries and Fileless Malware Execution Chains",
                    "Attackers leverage legitimate system tools like PowerShell and WMI to execute payloads without writing files to disk. Memory-resident implants evade signature-based detection, requiring behavioral analysis of process lineage.",
                ),
                (
                    "cryptographic ceremony design for mutual authentication",
                    "Formal Verification of Authentication Protocols Using ProVerif",
                    "Symbolic models enumerate attacker capabilities under the Dolev-Yao threat model. Automated provers verify secrecy and authentication properties, identifying protocol flaws before deployment.",
                ),
                (
                    "automated discovery of memory corruption vulnerabilities",
                    "Coverage-Guided Fuzzing and Symbolic Execution for Bug Finding",
                    "Grammar-aware fuzzers mutate structured inputs to explore program paths. Hybrid approaches combine concrete and symbolic execution to penetrate deep conditional branches guarding vulnerable code.",
                ),
                (
                    "adversarial machine learning in threat detection systems",
                    "Evasion Attacks Against Network Intrusion Detection Classifiers",
                    "Gradient-based adversarial examples modify network traffic features to avoid detection while preserving malicious functionality. Adversarial training and ensemble methods improve classifier robustness against adaptive attackers.",
                ),
                (
                    "supply chain compromise detection in software delivery",
                    "Software Bill of Materials and Provenance Verification with Sigstore",
                    "SBOM documents enumerate transitive dependencies for vulnerability tracking. Sigstore provides keyless signing using OIDC identity, creating verifiable provenance records in an immutable transparency log.",
                ),
            ],
        },
        TopicTemplates {
            topic: "ai_ml",
            easy: &[
                (
                    "neural network deep learning",
                    "Neural Networks and Deep Learning Basics",
                    "Neural networks are computing systems inspired by biological neurons. Deep learning uses many-layered networks to learn complex patterns from data for tasks like image recognition and natural language processing.",
                ),
                (
                    "large language model GPT",
                    "Large Language Models Like GPT",
                    "Large language models are trained on massive text datasets to generate human-like text. GPT and similar models use transformer architecture with self-attention mechanisms to process and generate language.",
                ),
                (
                    "machine learning model training",
                    "Training Machine Learning Models",
                    "Machine learning model training involves feeding labeled data through an algorithm that adjusts internal parameters to minimize prediction error. Common approaches include supervised, unsupervised, and reinforcement learning.",
                ),
                (
                    "computer vision image recognition",
                    "Computer Vision and Image Recognition",
                    "Computer vision systems use convolutional neural networks to analyze images. Applications include facial recognition, medical image analysis, autonomous driving, and quality inspection.",
                ),
                (
                    "recommendation system algorithms",
                    "How Recommendation System Algorithms Work",
                    "Recommendation systems suggest content based on user behavior. Collaborative filtering finds similar users, content-based filtering matches item features, and hybrid approaches combine both methods.",
                ),
            ],
            medium: &[
                (
                    "multi-layered pattern recognition systems",
                    "Backpropagation and Gradient Flow in Deep Architectures",
                    "Chain rule derivatives propagate loss gradients backward through network layers. Batch normalization and residual connections mitigate vanishing gradients, enabling training of networks with hundreds of layers.",
                ),
                (
                    "generative text models at billion-parameter scale",
                    "Attention Mechanisms and Context Window Scaling in Transformers",
                    "Scaled dot-product attention computes token relationships across the input sequence. Techniques like rotary positional embeddings and flash attention extend context length while controlling memory consumption.",
                ),
                (
                    "optimizing predictive model parameters from data",
                    "Stochastic Gradient Descent Variants and Learning Rate Scheduling",
                    "Adam optimizer combines momentum and adaptive learning rates per parameter. Cosine annealing schedules reduce the learning rate following a cosine curve, improving convergence to flatter minima.",
                ),
                (
                    "automated visual understanding systems",
                    "Feature Pyramid Networks and Multi-Scale Object Detection",
                    "Feature pyramids combine high-resolution low-level features with semantically rich high-level features. Anchor-free detectors directly predict bounding box parameters at each spatial location.",
                ),
                (
                    "personalized content suggestion engines",
                    "Matrix Factorization and Neural Collaborative Filtering",
                    "Embedding layers represent users and items as dense vectors in a shared latent space. Interaction modeling captures non-linear preference patterns that linear matrix factorization cannot express.",
                ),
            ],
            hard: &[
                (
                    "loss landscape geometry and generalization bounds",
                    "Sharpness-Aware Minimization and Flat Minima Hypothesis",
                    "SAM perturbs weights to maximize loss before taking a gradient step, biasing optimization toward flat regions. PAC-Bayes bounds relate the sharpness of the loss surface to generalization gap.",
                ),
                (
                    "scaling laws governing model capability emergence",
                    "Chinchilla Optimal Training and Compute-Efficient Frontier Analysis",
                    "Training loss follows a power law in both model parameters and data tokens. Compute-optimal allocation balances model size against dataset size to minimize loss per FLOP.",
                ),
                (
                    "distributional robustness in deployed prediction systems",
                    "Domain Adaptation and Covariate Shift Correction Under Distribution Mismatch",
                    "Importance weighting reweights training examples to match the target distribution. Domain-adversarial neural networks learn representations that are invariant to the source-target domain distinction.",
                ),
                (
                    "compositional scene understanding beyond bounding boxes",
                    "Panoptic Segmentation and Unified Stuff-Thing Representations",
                    "Mask transformers predict class-conditional binary masks for both countable objects and amorphous regions. End-to-end architectures eliminate the need for separate detection and segmentation pipelines.",
                ),
                (
                    "alignment of language model behavior with human values",
                    "RLHF and Constitutional AI Approaches to Value Alignment",
                    "Reward models trained on human preference comparisons provide optimization signal for policy gradient methods. Constitutional approaches use self-critique and revision to reduce harmful outputs without extensive human labeling.",
                ),
            ],
        },
        TopicTemplates {
            topic: "blockchain",
            easy: &[
                (
                    "bitcoin proof of work mining",
                    "Bitcoin Proof of Work Mining",
                    "Bitcoin mining uses proof of work consensus where miners solve cryptographic puzzles to validate transactions and create new blocks. The first miner to find a valid hash earns block rewards.",
                ),
                (
                    "ethereum smart contracts",
                    "Ethereum Smart Contract Development",
                    "Smart contracts are self-executing programs stored on the Ethereum blockchain. Written in Solidity, they automatically enforce agreement terms when predefined conditions are met.",
                ),
                (
                    "cryptocurrency wallet security",
                    "Cryptocurrency Wallet Security Best Practices",
                    "Cryptocurrency wallets store private keys that control digital assets. Hardware wallets provide cold storage offline, while software wallets offer convenience with encryption and backup seed phrases.",
                ),
                (
                    "DeFi decentralized finance protocols",
                    "Decentralized Finance DeFi Protocols",
                    "DeFi protocols enable lending, borrowing, trading, and yield farming without traditional intermediaries. Smart contracts on blockchain networks like Ethereum automate financial operations transparently.",
                ),
                (
                    "blockchain consensus mechanisms",
                    "Blockchain Consensus Mechanism Comparison",
                    "Consensus mechanisms ensure blockchain network agreement. Proof of Work uses computational power, Proof of Stake uses staked tokens, and delegated variants use elected validators.",
                ),
            ],
            medium: &[
                (
                    "computational puzzle-based transaction validation",
                    "Hashrate Security and Selfish Mining Strategies",
                    "Network hashrate determines the cost of a 51% attack. Selfish mining withholds discovered blocks to gain disproportionate revenue, undermining the incentive compatibility of Nakamoto consensus.",
                ),
                (
                    "programmable agreements on distributed ledgers",
                    "EVM Bytecode Execution and Gas Metering",
                    "The Ethereum Virtual Machine executes stack-based bytecode with deterministic gas costs per opcode. Gas limits prevent infinite loops while gas pricing allocates scarce block space through fee markets.",
                ),
                (
                    "safeguarding digital asset private keys",
                    "Hierarchical Deterministic Key Derivation and BIP-39 Mnemonics",
                    "A single master seed derives an entire tree of key pairs through HMAC-SHA512 chain codes. Mnemonic phrases encode the seed as human-readable words with built-in checksum verification.",
                ),
                (
                    "automated financial services on public blockchains",
                    "Automated Market Makers and Constant Product Liquidity Pools",
                    "Uniswap's x*y=k invariant prices assets based on pool reserves without order books. Concentrated liquidity positions allocate capital to specific price ranges, improving capital efficiency.",
                ),
                (
                    "distributed agreement protocols for shared state",
                    "Byzantine Fault Tolerance and Finality Guarantees in PoS Chains",
                    "Tendermint-style BFT achieves instant finality with two-thirds honest validator assumption. Slashing conditions penalize equivocation and inactivity, aligning validator incentives with protocol security.",
                ),
            ],
            hard: &[
                (
                    "game-theoretic security of permissionless ledgers",
                    "Miner Extractable Value and Transaction Ordering Manipulation",
                    "Validators profit by reordering, inserting, or censoring transactions within blocks. Proposer-builder separation and encrypted mempools mitigate MEV extraction that harms ordinary users.",
                ),
                (
                    "formal correctness of on-chain program logic",
                    "Symbolic Execution and Invariant Verification for Smart Contracts",
                    "Tools like Certora and Halmos explore all reachable program states to verify user-specified invariants. Formal methods detect reentrancy, integer overflow, and access control vulnerabilities before deployment.",
                ),
                (
                    "cryptographic primitives enabling private transactions",
                    "Zero-Knowledge Proofs and zk-SNARK Circuit Construction",
                    "Arithmetic circuits encode computation as polynomial constraints over finite fields. Groth16 and PLONK proving systems generate succinct proofs that verify in constant time regardless of circuit size.",
                ),
                (
                    "systemic risk in composable on-chain financial systems",
                    "Oracle Manipulation and Flash Loan Attack Vectors in DeFi",
                    "Atomic transaction composition enables capital-free exploitation of price oracle lag. Time-weighted average price oracles and multi-source aggregation increase manipulation cost but introduce latency trade-offs.",
                ),
                (
                    "economic incentive design for decentralized protocols",
                    "Token Curated Registries and Schelling Point Mechanisms",
                    "Staking tokens to vouch for registry entries creates skin-in-the-game incentives for curation quality. Challenge mechanisms leverage focal point coordination to resolve disputes without centralized arbitration.",
                ),
            ],
        },
        TopicTemplates {
            topic: "space",
            easy: &[
                (
                    "Mars rover exploration missions",
                    "Mars Rover Exploration Missions",
                    "NASA's Mars rovers including Spirit, Opportunity, Curiosity, and Perseverance have explored the Martian surface. They analyze soil samples, photograph terrain, and search for signs of past microbial life.",
                ),
                (
                    "International Space Station research",
                    "Research on the International Space Station",
                    "The International Space Station orbits Earth at 400 km altitude, hosting experiments in microgravity. Research covers biology, materials science, fluid dynamics, and Earth observation.",
                ),
                (
                    "SpaceX reusable rocket technology",
                    "SpaceX Reusable Rocket Technology",
                    "SpaceX developed reusable Falcon 9 first stages that land vertically after launch. Reusability has dramatically reduced launch costs and increased launch frequency.",
                ),
                (
                    "James Webb Space Telescope discoveries",
                    "James Webb Space Telescope Discoveries",
                    "The James Webb Space Telescope observes in infrared wavelengths, capturing images of early galaxies, exoplanet atmospheres, and stellar nurseries with unprecedented resolution and sensitivity.",
                ),
                (
                    "satellite constellation orbit design",
                    "Satellite Constellation Orbit Design",
                    "Satellite constellations use multiple spacecraft in coordinated orbits to provide continuous global coverage. Walker delta patterns distribute satellites evenly across orbital planes.",
                ),
            ],
            medium: &[
                (
                    "robotic surface exploration of other planets",
                    "Entry Descent and Landing Sequences for Planetary Rovers",
                    "Atmospheric entry generates extreme heating requiring ablative heat shields. Supersonic parachutes and powered descent stages decelerate the vehicle for a soft landing on the target surface.",
                ),
                (
                    "long-duration human habitation in orbit",
                    "Countermeasures for Bone Loss and Muscle Atrophy in Microgravity",
                    "Resistive exercise protocols and bisphosphonate therapy mitigate skeletal demineralization. Two hours of daily exercise on specialized equipment partially preserves musculoskeletal function during six-month missions.",
                ),
                (
                    "vertical landing rocket stages for cost reduction",
                    "Propulsive Landing Guidance and Grid Fin Aerodynamic Control",
                    "Convex optimization algorithms compute fuel-optimal trajectories in real time. Grid fins provide aerodynamic steering during supersonic descent while engine gimbaling controls the terminal landing burn.",
                ),
                (
                    "space-based infrared astronomical observatory",
                    "Cryogenic Mirror Alignment and Mid-Infrared Detector Sensitivity",
                    "Beryllium primary mirror segments are aligned to nanometer precision using wavefront sensing. Mercury-cadmium-telluride detectors cooled to 6 Kelvin achieve the sensitivity needed for distant galaxy observation.",
                ),
                (
                    "multi-spacecraft coordinated orbital systems",
                    "Orbital Mechanics and Station-Keeping Propulsion Budgets",
                    "Atmospheric drag at LEO altitudes requires periodic orbit-raising maneuvers. Electric propulsion systems provide high specific impulse for station-keeping, extending constellation operational lifetime.",
                ),
            ],
            hard: &[
                (
                    "autonomous decision-making for deep space exploration",
                    "Onboard Fault Detection and Multi-Agent Planning for Interplanetary Missions",
                    "Communication latency of up to 24 minutes at Mars opposition precludes real-time ground control. Onboard planning systems autonomously sequence science observations and manage faults using model-based reasoning.",
                ),
                (
                    "radiation exposure limits for interplanetary crews",
                    "Galactic Cosmic Ray Shielding and Stochastic Risk Assessment Models",
                    "High-energy heavy ions penetrate spacecraft structures, depositing dose along ionization tracks. NASA's NSCR model estimates career cancer risk using quality factors that account for ion species and energy.",
                ),
                (
                    "turbo-pump engineering for liquid-fueled rocket engines",
                    "Regenerative Cooling Channel Design and Combustion Instability Suppression",
                    "Fuel circulates through channels machined into the chamber wall, absorbing heat before injection. Acoustic dampeners and baffles suppress longitudinal combustion modes that can destroy engine hardware.",
                ),
                (
                    "coronagraphic techniques for direct exoplanet imaging",
                    "Starshade Formation Flying and Inner Working Angle Limitations",
                    "External occulters positioned tens of thousands of kilometers from a telescope suppress stellar diffraction. Petal-shaped starshade edges minimize Poisson spot intensity, achieving contrast ratios of 10^-10.",
                ),
                (
                    "debris collision risk in increasingly crowded orbits",
                    "Conjunction Assessment and Collision Avoidance Maneuver Planning",
                    "Propagated covariance matrices estimate collision probability from tracking data. When probability exceeds thresholds, operators execute avoidance maneuvers balancing fuel expenditure against residual risk.",
                ),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Adversarial templates -- spam / slop patterns
// ---------------------------------------------------------------------------

fn adversarial_templates() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "keyword stuffing tech spam",
            "Best Best Best Technology Technology Technology",
            "Technology technology technology best best best technology guide technology review technology technology technology tips technology news technology.",
        ),
        (
            "keyword stuffing finance spam",
            "Money Money Money Finance Finance Tips",
            "Finance finance finance money money money invest invest invest finance tips finance finance finance guide finance finance.",
        ),
        (
            "keyword stuffing health spam",
            "Health Health Health Wellness Wellness Tips",
            "Health health health wellness wellness diet diet supplement supplement health tips health health health guide health wellness.",
        ),
        (
            "keyword stuffing SEO spam",
            "SEO SEO SEO Rankings Rankings Best Best",
            "SEO SEO SEO rankings rankings best best optimization optimization SEO tips SEO guide SEO SEO SEO rankings.",
        ),
        (
            "keyword stuffing crypto spam",
            "Crypto Crypto Bitcoin Bitcoin Moon Moon Gains",
            "Crypto crypto crypto bitcoin bitcoin moon moon gains gains crypto invest crypto crypto crypto lambo crypto gains.",
        ),
        (
            "ai slop technology landscape",
            "Navigating the Digital Landscape",
            "In today's rapidly evolving technological landscape, it is important to note that the intersection of innovation and disruption continues to reshape how we think about digital transformation.",
        ),
        (
            "ai slop comprehensive guide",
            "A Comprehensive Guide to Excellence",
            "In this comprehensive guide, we will delve into the intricacies of this multifaceted topic. It goes without saying that understanding these nuances is crucial for stakeholders.",
        ),
        (
            "ai slop thought leadership",
            "Unlocking the Power of Synergy",
            "As we stand at the precipice of unprecedented change, it is imperative that we leverage cutting-edge solutions to unlock transformative value for all stakeholders in the ecosystem.",
        ),
        (
            "ai slop paradigm shift",
            "Revolutionizing the Paradigm",
            "This groundbreaking approach represents a paradigm shift in how we conceptualize innovation. The synergistic integration of best-in-class methodologies drives holistic transformation.",
        ),
        (
            "ai slop game changer",
            "The Game-Changing Solution You Need",
            "This game-changing solution is disrupting the industry in unprecedented ways. Leveraging state-of-the-art technology, it empowers organizations to achieve next-level outcomes at scale.",
        ),
        (
            "low info repetitive content",
            "Important Information About Things",
            "This is important. This is very important. You should know this. It is important to understand this. Understanding this is important. This is a key point.",
        ),
        (
            "low info filler text",
            "Everything You Need to Know",
            "There are many things to consider. Many things are important. It is good to know things. Knowing things helps you. Things are useful to know about. Consider many things.",
        ),
        (
            "low info circular reasoning",
            "Understanding the Basics",
            "The basics are basic because they are fundamental. Fundamentals are fundamental because they form the basis. The basis is basic because it is fundamental to the basics.",
        ),
        (
            "low info vague promises",
            "Amazing Results Guaranteed",
            "You will see amazing results. These results will amaze you. The amazing results speak for themselves. Results that are truly amazing await you. Amazing.",
        ),
        (
            "low info clickbait",
            "You Won't Believe What Happened Next",
            "What happened next was unbelievable. The shocking truth will surprise you. Number seven will blow your mind. Experts are stunned by this revelation.",
        ),
        (
            "spam promotional discount",
            "LIMITED TIME OFFER BUY NOW",
            "AMAZING DEAL! BUY NOW before it's too late! 90% OFF for a LIMITED TIME! Click here immediately! Don't miss out on this INCREDIBLE opportunity! ACT FAST!",
        ),
        (
            "spam fake reviews",
            "5 Stars Best Product Ever Amazing",
            "Best product ever! Five stars! Amazing quality! Would recommend to everyone! Changed my life! Incredible value! Must buy! Perfect in every way! Love it!",
        ),
        (
            "spam link farm",
            "Click Here Free Download Links",
            "Free download link click here. More free links here. Click for free. Another free link. Best free links collection. Click all links for free.",
        ),
        (
            "spam engagement bait",
            "Like Share Subscribe Follow Now",
            "Like and subscribe! Share with friends! Follow for more! Hit the bell! Don't forget to like! Share share share! Subscribe now! Follow us!",
        ),
        (
            "spam miracle cure",
            "Doctors Hate This One Simple Trick",
            "Doctors are FURIOUS about this simple trick. Big pharma doesn't want you to know. This ancient secret cures everything. One weird trick that changes everything.",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Public generation entry point
// ---------------------------------------------------------------------------

/// Generate a benchmark dataset with the given number of cases.
///
/// Distribution (scaled proportionally to `count`):
/// - Easy:         30 %
/// - Medium:       30 %
/// - Hard:         20 %
/// - Adversarial:  20 %
///
/// Uses deterministic seeding via SHA-256 so the output is reproducible.
pub fn generate_text_dataset(count: usize) -> BenchmarkDataset {
    let easy_count = (count as f64 * 0.30).round() as usize;
    let medium_count = (count as f64 * 0.30).round() as usize;
    let hard_count = (count as f64 * 0.20).round() as usize;
    let adversarial_count = count.saturating_sub(easy_count + medium_count + hard_count);

    let templates = all_templates();
    let adversarial_tpls = adversarial_templates();

    let mut cases: Vec<BenchmarkCase> = Vec::with_capacity(count);
    let mut global_id: usize = 0;

    // Helper: generate cases for a given difficulty
    let mut gen_cases = |difficulty: Difficulty, n: usize| {
        let seed_str = format!("opensonarx-bench-{difficulty}-v1");
        let mut rng = Rng::new(hash_seed(&seed_str));

        for i in 0..n {
            let case_seed = format!("{seed_str}-case-{i}");
            let mut case_rng = Rng::new(hash_seed(&case_seed));

            let topic_idx = case_rng.usize_range(0, templates.len());
            let tpl = &templates[topic_idx];

            let pairs = match difficulty {
                Difficulty::Easy => tpl.easy,
                Difficulty::Medium => tpl.medium,
                Difficulty::Hard => tpl.hard,
                Difficulty::Adversarial => unreachable!(),
            };
            let pair_idx = case_rng.usize_range(0, pairs.len());
            let (query, title, content) = pairs[pair_idx];

            let region = rng.pick(REGIONS);
            let content_type = rng.pick(CONTENT_TYPES);

            // Build relevant docs with graded relevance
            let mut relevant_docs = Vec::new();

            // Primary doc -- perfect match (relevance 3)
            relevant_docs.push(RelevantDoc {
                doc_id: format!("doc-{global_id}-rel3"),
                title: title.to_string(),
                content: content.to_string(),
                relevance: 3,
                image_description: None,
            });

            // Good match (relevance 2) -- pick another pair from same topic/difficulty
            let alt_idx = (pair_idx + 1) % pairs.len();
            let (_, alt_title, alt_content) = pairs[alt_idx];
            relevant_docs.push(RelevantDoc {
                doc_id: format!("doc-{global_id}-rel2"),
                title: alt_title.to_string(),
                content: alt_content.to_string(),
                relevance: 2,
                image_description: None,
            });

            // Partial match (relevance 1) -- different difficulty, same topic
            let partial_pairs = match difficulty {
                Difficulty::Easy => tpl.medium,
                Difficulty::Medium => tpl.hard,
                Difficulty::Hard => tpl.easy,
                Difficulty::Adversarial => unreachable!(),
            };
            let partial_idx = case_rng.usize_range(0, partial_pairs.len());
            let (_, p_title, p_content) = partial_pairs[partial_idx];
            relevant_docs.push(RelevantDoc {
                doc_id: format!("doc-{global_id}-rel1"),
                title: p_title.to_string(),
                content: p_content.to_string(),
                relevance: 1,
                image_description: None,
            });

            // Not relevant (relevance 0) -- different topic entirely
            let other_topic_idx = (topic_idx + 1) % templates.len();
            let other_tpl = &templates[other_topic_idx];
            let other_pairs = other_tpl.easy;
            let other_idx = case_rng.usize_range(0, other_pairs.len());
            let (_, o_title, o_content) = other_pairs[other_idx];
            relevant_docs.push(RelevantDoc {
                doc_id: format!("doc-{global_id}-rel0"),
                title: o_title.to_string(),
                content: o_content.to_string(),
                relevance: 0,
                image_description: None,
            });

            cases.push(BenchmarkCase {
                id: format!("case-{global_id:05}"),
                query: query.to_string(),
                difficulty,
                relevant_docs,
                metadata: CaseMetadata {
                    region: region.to_string(),
                    language: "en".to_string(),
                    topic: tpl.topic.to_string(),
                    content_type: content_type.to_string(),
                    modality: "text-to-text".to_string(),
                },
            });

            global_id += 1;
        }
    };

    gen_cases(Difficulty::Easy, easy_count);
    gen_cases(Difficulty::Medium, medium_count);
    gen_cases(Difficulty::Hard, hard_count);

    // Adversarial cases
    {
        let seed_str = "opensonarx-bench-adversarial-v1";
        let mut rng = Rng::new(hash_seed(seed_str));

        for i in 0..adversarial_count {
            let case_seed = format!("{seed_str}-case-{i}");
            let mut case_rng = Rng::new(hash_seed(&case_seed));

            let tpl_idx = case_rng.usize_range(0, adversarial_tpls.len());
            let (query, title, content) = adversarial_tpls[tpl_idx];

            let topic_idx = case_rng.usize_range(0, TOPICS.len());
            let region = rng.pick(REGIONS);
            let content_type = rng.pick(CONTENT_TYPES);

            // Adversarial docs -- ALL relevance 0
            let mut relevant_docs = vec![RelevantDoc {
                doc_id: format!("doc-{global_id}-adv0"),
                title: title.to_string(),
                content: content.to_string(),
                relevance: 0,
                image_description: None,
            }];

            // Add a few more adversarial docs from different templates
            for j in 1..=3usize {
                let other_idx = (tpl_idx + j) % adversarial_tpls.len();
                let (_, ot, oc) = adversarial_tpls[other_idx];
                relevant_docs.push(RelevantDoc {
                    doc_id: format!("doc-{global_id}-adv{j}"),
                    title: ot.to_string(),
                    content: oc.to_string(),
                    relevance: 0,
                    image_description: None,
                });
            }

            cases.push(BenchmarkCase {
                id: format!("case-{global_id:05}"),
                query: query.to_string(),
                difficulty: Difficulty::Adversarial,
                relevant_docs,
                metadata: CaseMetadata {
                    region: region.to_string(),
                    language: "en".to_string(),
                    topic: TOPICS[topic_idx].to_string(),
                    content_type: content_type.to_string(),
                    modality: "text-to-text".to_string(),
                },
            });

            global_id += 1;
        }
    }

    BenchmarkDataset {
        name: format!("opensonarx-text-{}k", count / 1000),
        description: format!(
            "OpenSonarX benchmark dataset with {} cases. Distribution: {} easy, {} medium, {} hard, {} adversarial.",
            cases.len(),
            easy_count,
            medium_count,
            hard_count,
            adversarial_count
        ),
        cases,
    }
}

/// Image search queries mapped to their relevant image templates.
const IMAGE_QUERIES: &[(&str, &str, &str)] = &[
    ("red square shape", "red_square", "blue_circle"),
    ("blue circle round shape", "blue_circle", "red_square"),
    ("green triangle geometry", "green_triangle", "checkerboard"),
    (
        "sunset gradient warm colors",
        "sunset_gradient",
        "solid_black",
    ),
    ("checkerboard pattern", "checkerboard", "stripes"),
    ("solid black dark image", "solid_black", "solid_white"),
    ("solid white blank image", "solid_white", "solid_black"),
    ("horizontal stripes pattern", "stripes", "dots"),
    ("dot pattern polka dots", "dots", "stripes"),
    ("diagonal lines pattern", "diagonal_lines", "checkerboard"),
];

/// Generate a cross-modal benchmark dataset (text query → image results).
pub fn generate_image_dataset(count: usize) -> BenchmarkDataset {
    let templates = image_gen::all_templates();
    let mut cases = Vec::with_capacity(count);

    for i in 0..count {
        let query_idx = i % IMAGE_QUERIES.len();
        let (query, relevant_label, partial_label) = IMAGE_QUERIES[query_idx];

        let relevant_tpl = templates
            .iter()
            .find(|t| t.label == relevant_label)
            .unwrap();
        let partial_tpl = templates.iter().find(|t| t.label == partial_label).unwrap();

        let relevant_doc = RelevantDoc {
            doc_id: format!("img-{i}-rel3"),
            title: relevant_tpl.description.clone(),
            content: relevant_tpl.description.clone(),
            relevance: 3,
            image_description: Some(relevant_tpl.description.clone()),
        };

        let partial_doc = RelevantDoc {
            doc_id: format!("img-{i}-rel1"),
            title: partial_tpl.description.clone(),
            content: partial_tpl.description.clone(),
            relevance: 1,
            image_description: Some(partial_tpl.description.clone()),
        };

        // Irrelevant text-only doc (not an image)
        let irrelevant_doc = RelevantDoc {
            doc_id: format!("img-{i}-rel0"),
            title: "Unrelated text document".to_string(),
            content:
                "This document discusses completely unrelated financial regulations and tax policy."
                    .to_string(),
            relevance: 0,
            image_description: None,
        };

        cases.push(BenchmarkCase {
            id: format!("img-case-{i:05}"),
            query: query.to_string(),
            difficulty: Difficulty::Easy,
            relevant_docs: vec![relevant_doc, partial_doc, irrelevant_doc],
            metadata: CaseMetadata {
                region: "global".to_string(),
                language: "en".to_string(),
                topic: "visual".to_string(),
                content_type: "image".to_string(),
                modality: "text-to-image".to_string(),
            },
        });
    }

    BenchmarkDataset {
        name: format!("opensonarx-image-{count}"),
        description: format!("Cross-modal image benchmark with {count} cases."),
        cases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_correct_count() {
        let ds = generate_text_dataset(100);
        assert_eq!(ds.cases.len(), 100);
    }

    #[test]
    fn correct_distribution_at_5k() {
        let ds = generate_text_dataset(5000);
        let easy = ds
            .cases
            .iter()
            .filter(|c| c.difficulty == Difficulty::Easy)
            .count();
        let medium = ds
            .cases
            .iter()
            .filter(|c| c.difficulty == Difficulty::Medium)
            .count();
        let hard = ds
            .cases
            .iter()
            .filter(|c| c.difficulty == Difficulty::Hard)
            .count();
        let adv = ds
            .cases
            .iter()
            .filter(|c| c.difficulty == Difficulty::Adversarial)
            .count();
        assert_eq!(easy, 1500);
        assert_eq!(medium, 1500);
        assert_eq!(hard, 1000);
        assert_eq!(adv, 1000);
    }

    #[test]
    fn deterministic_output() {
        let a = generate_text_dataset(50);
        let b = generate_text_dataset(50);
        assert_eq!(a.cases.len(), b.cases.len());
        for (ca, cb) in a.cases.iter().zip(b.cases.iter()) {
            assert_eq!(ca.id, cb.id);
            assert_eq!(ca.query, cb.query);
        }
    }

    #[test]
    fn adversarial_cases_have_zero_relevance() {
        let ds = generate_text_dataset(200);
        for case in &ds.cases {
            if case.difficulty == Difficulty::Adversarial {
                for doc in &case.relevant_docs {
                    assert_eq!(
                        doc.relevance, 0,
                        "Adversarial case {} doc {} should have relevance 0",
                        case.id, doc.doc_id
                    );
                }
            }
        }
    }

    #[test]
    fn all_twenty_topics_covered() {
        let ds = generate_text_dataset(5000);
        let mut seen = std::collections::HashSet::new();
        for case in &ds.cases {
            seen.insert(case.metadata.topic.clone());
        }
        for topic in TOPICS {
            assert!(
                seen.contains(*topic),
                "Topic {} not found in dataset",
                topic
            );
        }
    }

    #[test]
    fn non_adversarial_have_graded_relevance() {
        let ds = generate_text_dataset(200);
        for case in &ds.cases {
            if case.difficulty != Difficulty::Adversarial {
                let max_rel = case
                    .relevant_docs
                    .iter()
                    .map(|d| d.relevance)
                    .max()
                    .unwrap_or(0);
                assert_eq!(max_rel, 3, "Case {} should have a relevance-3 doc", case.id);
                let has_zero = case.relevant_docs.iter().any(|d| d.relevance == 0);
                assert!(has_zero, "Case {} should have a relevance-0 doc", case.id);
            }
        }
    }

    #[test]
    fn dataset_serializes_to_json() {
        let ds = generate_text_dataset(50);
        let json = serde_json::to_string(&ds).unwrap();
        let parsed: BenchmarkDataset = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.cases.len(), 50);
    }

    #[test]
    fn generates_image_dataset() {
        let ds = generate_image_dataset(20);
        assert_eq!(ds.cases.len(), 20);
        for case in &ds.cases {
            assert_eq!(case.metadata.modality, "text-to-image");
            // Should have at least one image doc
            let has_image = case
                .relevant_docs
                .iter()
                .any(|d| d.image_description.is_some());
            assert!(has_image, "Case {} should have image docs", case.id);
        }
    }
}
