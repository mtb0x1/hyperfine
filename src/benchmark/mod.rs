pub mod benchmark_result;
pub mod executor;
pub mod relative_speed;
pub mod scheduler;
pub mod timing_result;

use std::cmp;

use crate::benchmark::executor::BenchmarkIteration;
use crate::command::Command;
use crate::options::{
    CmdFailureAction, CommandOutputPolicy, ExecutorKind, Options, OutputStyleOption,
};
use crate::outlier_detection::{modified_zscores, OUTLIER_THRESHOLD};
use crate::output::format::{format_duration, format_duration_unit};
use crate::output::progress_bar::get_progress_bar;
use crate::output::warnings::{OutlierWarningOptions, Warnings};
use crate::parameter::ParameterNameAndValue;
use crate::poop_metrics::PoopMetrics;
use crate::util::exit_code::extract_exit_code;
use crate::util::min_max::{max, min};
use crate::util::units::Second;
use benchmark_result::BenchmarkResult;
use timing_result::TimingResult;

use anyhow::{anyhow, Result};
use colored::*;
use statistical::{mean, median, standard_deviation};

use self::executor::Executor;

/// Aggregate poop metrics from multiple timing results
fn aggregate_poop_metrics(timing_results: &[TimingResult]) -> Option<PoopMetrics> {
    let metrics_with_data: Vec<&PoopMetrics> = timing_results
        .iter()
        .filter_map(|tr| tr.poop_metrics.as_ref())
        .collect();

    if metrics_with_data.is_empty() {
        return None;
    }

    let mut aggregated = PoopMetrics::new();

    // Count how many results have each metric
    let cpu_cycles_count = metrics_with_data
        .iter()
        .filter(|m| m.cpu_cycles.is_some())
        .count();
    let instructions_count = metrics_with_data
        .iter()
        .filter(|m| m.instructions.is_some())
        .count();
    let cache_refs_count = metrics_with_data
        .iter()
        .filter(|m| m.cache_references.is_some())
        .count();
    let cache_misses_count = metrics_with_data
        .iter()
        .filter(|m| m.cache_misses.is_some())
        .count();
    let branches_count = metrics_with_data
        .iter()
        .filter(|m| m.branches.is_some())
        .count();
    let branch_misses_count = metrics_with_data
        .iter()
        .filter(|m| m.branch_misses.is_some())
        .count();
    let page_faults_count = metrics_with_data
        .iter()
        .filter(|m| m.page_faults.is_some())
        .count();

    // Aggregate each metric
    if cpu_cycles_count > 0 {
        let sum: u64 = metrics_with_data.iter().filter_map(|m| m.cpu_cycles).sum();
        aggregated.cpu_cycles = Some(sum / cpu_cycles_count as u64);
    }
    if instructions_count > 0 {
        let sum: u64 = metrics_with_data
            .iter()
            .filter_map(|m| m.instructions)
            .sum();
        aggregated.instructions = Some(sum / instructions_count as u64);
    }
    if cache_refs_count > 0 {
        let sum: u64 = metrics_with_data
            .iter()
            .filter_map(|m| m.cache_references)
            .sum();
        aggregated.cache_references = Some(sum / cache_refs_count as u64);
    }
    if cache_misses_count > 0 {
        let sum: u64 = metrics_with_data
            .iter()
            .filter_map(|m| m.cache_misses)
            .sum();
        aggregated.cache_misses = Some(sum / cache_misses_count as u64);
    }
    if branches_count > 0 {
        let sum: u64 = metrics_with_data.iter().filter_map(|m| m.branches).sum();
        aggregated.branches = Some(sum / branches_count as u64);
    }
    if branch_misses_count > 0 {
        let sum: u64 = metrics_with_data
            .iter()
            .filter_map(|m| m.branch_misses)
            .sum();
        aggregated.branch_misses = Some(sum / branch_misses_count as u64);
    }
    if page_faults_count > 0 {
        let sum: u64 = metrics_with_data.iter().filter_map(|m| m.page_faults).sum();
        aggregated.page_faults = Some(sum / page_faults_count as u64);
    }

    Some(aggregated)
}

/// Threshold for warning about fast execution time
pub const MIN_EXECUTION_TIME: Second = 5e-3;

pub struct Benchmark<'a> {
    number: usize,
    command: &'a Command<'a>,
    options: &'a Options,
    executor: &'a dyn Executor,
}

impl<'a> Benchmark<'a> {
    pub fn new(
        number: usize,
        command: &'a Command<'a>,
        options: &'a Options,
        executor: &'a dyn Executor,
    ) -> Self {
        Benchmark {
            number,
            command,
            options,
            executor,
        }
    }

    /// Run setup, cleanup, or preparation commands
    fn run_intermediate_command(
        &self,
        command: &Command<'_>,
        error_output: &'static str,
        output_policy: &CommandOutputPolicy,
    ) -> Result<TimingResult> {
        self.executor
            .run_command_and_measure(
                command,
                executor::BenchmarkIteration::NonBenchmarkRun,
                Some(CmdFailureAction::RaiseError),
                output_policy,
            )
            .map(|r| r.0)
            .map_err(|_| anyhow!(error_output))
    }

    /// Run the command specified by `--setup`.
    fn run_setup_command(
        &self,
        parameters: impl IntoIterator<Item = ParameterNameAndValue<'a>>,
        output_policy: &CommandOutputPolicy,
    ) -> Result<TimingResult> {
        let command = self
            .options
            .setup_command
            .as_ref()
            .map(|setup_command| Command::new_parametrized(None, setup_command, parameters));

        let error_output = "The setup command terminated with a non-zero exit code. \
                            Append ' || true' to the command if you are sure that this can be ignored.";

        Ok(command
            .map(|cmd| self.run_intermediate_command(&cmd, error_output, output_policy))
            .transpose()?
            .unwrap_or_default())
    }

    /// Run the command specified by `--cleanup`.
    fn run_cleanup_command(
        &self,
        parameters: impl IntoIterator<Item = ParameterNameAndValue<'a>>,
        output_policy: &CommandOutputPolicy,
    ) -> Result<TimingResult> {
        let command = self
            .options
            .cleanup_command
            .as_ref()
            .map(|cleanup_command| Command::new_parametrized(None, cleanup_command, parameters));

        let error_output = "The cleanup command terminated with a non-zero exit code. \
                            Append ' || true' to the command if you are sure that this can be ignored.";

        Ok(command
            .map(|cmd| self.run_intermediate_command(&cmd, error_output, output_policy))
            .transpose()?
            .unwrap_or_default())
    }

    /// Run the command specified by `--prepare`.
    fn run_preparation_command(
        &self,
        command: &Command<'_>,
        output_policy: &CommandOutputPolicy,
    ) -> Result<TimingResult> {
        let error_output = "The preparation command terminated with a non-zero exit code. \
                            Append ' || true' to the command if you are sure that this can be ignored.";

        self.run_intermediate_command(command, error_output, output_policy)
    }

    /// Run the command specified by `--conclude`.
    fn run_conclusion_command(
        &self,
        command: &Command<'_>,
        output_policy: &CommandOutputPolicy,
    ) -> Result<TimingResult> {
        let error_output = "The conclusion command terminated with a non-zero exit code. \
                            Append ' || true' to the command if you are sure that this can be ignored.";

        self.run_intermediate_command(command, error_output, output_policy)
    }

    /// Run the benchmark for a single command
    pub fn run(&self) -> Result<BenchmarkResult> {
        if self.options.output_style != OutputStyleOption::Disabled {
            println!(
                "{}{}: {}",
                "Benchmark ".bold(),
                (self.number + 1).to_string().bold(),
                self.command.get_name_with_unused_parameters(),
            );
        }

        let mut times_real: Vec<Second> = vec![];
        let mut times_user: Vec<Second> = vec![];
        let mut times_system: Vec<Second> = vec![];
        let mut memory_usage_byte: Vec<u64> = vec![];
        let mut exit_codes: Vec<Option<i32>> = vec![];
        let mut timing_results: Vec<TimingResult> = vec![];
        let mut all_succeeded = true;

        let output_policy = &self.options.command_output_policies[self.number];

        let preparation_command = self.options.preparation_command.as_ref().map(|values| {
            let preparation_command = if values.len() == 1 {
                &values[0]
            } else {
                &values[self.number]
            };
            Command::new_parametrized(
                None,
                preparation_command,
                self.command.get_parameters().iter().cloned(),
            )
        });

        let run_preparation_command = || {
            preparation_command
                .as_ref()
                .map(|cmd| self.run_preparation_command(cmd, output_policy))
                .transpose()
        };

        let conclusion_command = self.options.conclusion_command.as_ref().map(|values| {
            let conclusion_command = if values.len() == 1 {
                &values[0]
            } else {
                &values[self.number]
            };
            Command::new_parametrized(
                None,
                conclusion_command,
                self.command.get_parameters().iter().cloned(),
            )
        });
        let run_conclusion_command = || {
            conclusion_command
                .as_ref()
                .map(|cmd| self.run_conclusion_command(cmd, output_policy))
                .transpose()
        };

        self.run_setup_command(self.command.get_parameters().iter().cloned(), output_policy)?;

        // Warmup phase
        if self.options.warmup_count > 0 {
            let progress_bar = if self.options.output_style != OutputStyleOption::Disabled {
                Some(get_progress_bar(
                    self.options.warmup_count,
                    "Performing warmup runs",
                    self.options.output_style,
                ))
            } else {
                None
            };

            for i in 0..self.options.warmup_count {
                let _ = run_preparation_command()?;
                let _ = self.executor.run_command_and_measure(
                    self.command,
                    BenchmarkIteration::Warmup(i),
                    None,
                    output_policy,
                )?;
                let _ = run_conclusion_command()?;
                if let Some(bar) = progress_bar.as_ref() {
                    bar.inc(1)
                }
            }
            if let Some(bar) = progress_bar.as_ref() {
                bar.finish_and_clear()
            }
        }

        // Set up progress bar (and spinner for initial measurement)
        let progress_bar = if self.options.output_style != OutputStyleOption::Disabled {
            Some(get_progress_bar(
                self.options.run_bounds.min,
                "Initial time measurement",
                self.options.output_style,
            ))
        } else {
            None
        };

        let preparation_result = run_preparation_command()?;
        let preparation_overhead =
            preparation_result.map_or(0.0, |res| res.time_real + self.executor.time_overhead());

        // Initial timing run
        let (res, status) = self.executor.run_command_and_measure(
            self.command,
            BenchmarkIteration::Benchmark(0),
            None,
            output_policy,
        )?;
        let success = status.success();

        let conclusion_result = run_conclusion_command()?;
        let conclusion_overhead =
            conclusion_result.map_or(0.0, |res| res.time_real + self.executor.time_overhead());

        // Determine number of benchmark runs
        let runs_in_min_time = (self.options.min_benchmarking_time
            / (res.time_real
                + self.executor.time_overhead()
                + preparation_overhead
                + conclusion_overhead)) as u64;

        let count = {
            let min = cmp::max(runs_in_min_time, self.options.run_bounds.min);

            self.options
                .run_bounds
                .max
                .as_ref()
                .map(|max| cmp::min(min, *max))
                .unwrap_or(min)
        };

        let count_remaining = count - 1;

        // Save the first result
        times_real.push(res.time_real);
        times_user.push(res.time_user);
        times_system.push(res.time_system);
        memory_usage_byte.push(res.memory_usage_byte);
        exit_codes.push(extract_exit_code(status));
        timing_results.push(res);

        all_succeeded = all_succeeded && success;

        // Re-configure the progress bar
        if let Some(bar) = progress_bar.as_ref() {
            bar.set_length(count)
        }
        if let Some(bar) = progress_bar.as_ref() {
            bar.inc(1)
        }

        // Gather statistics (perform the actual benchmark)
        for i in 0..count_remaining {
            run_preparation_command()?;

            let msg = {
                let mean = format_duration(mean(&times_real), self.options.time_unit);
                format!("Current estimate: {}", mean.to_string().green())
            };

            if let Some(bar) = progress_bar.as_ref() {
                bar.set_message(msg.to_owned())
            }

            let (res, status) = self.executor.run_command_and_measure(
                self.command,
                BenchmarkIteration::Benchmark(i + 1),
                None,
                output_policy,
            )?;
            let success = status.success();

            times_real.push(res.time_real);
            times_user.push(res.time_user);
            times_system.push(res.time_system);
            memory_usage_byte.push(res.memory_usage_byte);
            exit_codes.push(extract_exit_code(status));
            timing_results.push(res);

            all_succeeded = all_succeeded && success;

            if let Some(bar) = progress_bar.as_ref() {
                bar.inc(1)
            }

            run_conclusion_command()?;
        }

        if let Some(bar) = progress_bar.as_ref() {
            bar.finish_and_clear()
        }

        // Compute statistical quantities
        let t_num = times_real.len();
        let t_mean = mean(&times_real);
        let t_stddev = if times_real.len() > 1 {
            Some(standard_deviation(&times_real, Some(t_mean)))
        } else {
            None
        };
        let t_median = median(&times_real);
        let t_min = min(&times_real);
        let t_max = max(&times_real);

        let user_mean = mean(&times_user);
        let system_mean = mean(&times_system);

        // Collect poop metrics for display
        let aggregated_poop_metrics = aggregate_poop_metrics(&timing_results);

        // Formatting and console output
        let (mean_str, time_unit) = format_duration_unit(t_mean, self.options.time_unit);
        let min_str = format_duration(t_min, Some(time_unit));
        let max_str = format_duration(t_max, Some(time_unit));
        let num_str = format!("{t_num} runs");

        let user_str = format_duration(user_mean, Some(time_unit));
        let system_str = format_duration(system_mean, Some(time_unit));

        if self.options.output_style != OutputStyleOption::Disabled {
            if times_real.len() == 1 {
                println!(
                    "  Time ({} ≡):        {:>8}  {:>8}     [User: {}, System: {}]",
                    "abs".green().bold(),
                    mean_str.green().bold(),
                    "        ", // alignment
                    user_str.blue(),
                    system_str.blue()
                );
            } else {
                let stddev_str = format_duration(t_stddev.unwrap(), Some(time_unit));

                println!(
                    "  Time ({} ± {}):     {:>8} ± {:>8}    [User: {}, System: {}]",
                    "mean".green().bold(),
                    "σ".green(),
                    mean_str.green().bold(),
                    stddev_str.green(),
                    user_str.blue(),
                    system_str.blue()
                );

                // Display poop metrics if collected
                if let Some(metrics) = aggregated_poop_metrics.as_ref() {
                    println!();
                    if let Some(cycles) = metrics.cpu_cycles {
                        print!("  CPU cycles:              {}", cycles.to_string().cyan());
                        if let Some(_instructions) = metrics.instructions {
                            if let Some(ipc) = metrics.instructions_per_cycle() {
                                println!("\t(IPC: {})", format!("{:.2}", ipc).cyan());
                            } else {
                                println!();
                            }
                        } else {
                            println!();
                        }
                    }
                    if let Some(instructions) = metrics.instructions {
                        println!(
                            "  Instructions:            {}",
                            instructions.to_string().cyan()
                        );
                    }
                    if let Some(cache_refs) = metrics.cache_references {
                        print!(
                            "  Cache references:        {}",
                            cache_refs.to_string().cyan()
                        );
                        if let Some(_cache_misses) = metrics.cache_misses {
                            if let Some(miss_rate) = metrics.cache_miss_rate() {
                                println!(
                                    "\t(miss rate: {}%)",
                                    format!("{:.1}", miss_rate * 100.0).yellow()
                                );
                            } else {
                                println!();
                            }
                        } else {
                            println!();
                        }
                    }
                    if let Some(cache_misses) = metrics.cache_misses {
                        if metrics.cache_references.is_none() {
                            println!(
                                "  Cache misses:            {}",
                                cache_misses.to_string().cyan()
                            );
                        }
                    }
                    if let Some(branches) = metrics.branches {
                        print!("  Branch instructions:     {}", branches.to_string().cyan());
                        if let Some(_branch_misses) = metrics.branch_misses {
                            if let Some(miss_rate) = metrics.branch_miss_rate() {
                                println!(
                                    "\t(miss rate: {}%)",
                                    format!("{:.1}", miss_rate * 100.0).yellow()
                                );
                            } else {
                                println!();
                            }
                        } else {
                            println!();
                        }
                    }
                    if let Some(branch_misses) = metrics.branch_misses {
                        if metrics.branches.is_none() {
                            println!(
                                "  Branch misses:           {}",
                                branch_misses.to_string().cyan()
                            );
                        }
                    }
                    if let Some(page_faults) = metrics.page_faults {
                        println!(
                            "  Page faults:             {}",
                            page_faults.to_string().cyan()
                        );
                    }
                }

                println!(
                    "  Range ({} … {}):   {:>8} … {:>8}    {}",
                    "min".cyan(),
                    "max".purple(),
                    min_str.cyan(),
                    max_str.purple(),
                    num_str.dimmed()
                );
            }
        }

        // Warnings
        let mut warnings = vec![];

        // Check execution time
        if matches!(self.options.executor_kind, ExecutorKind::Shell(_))
            && times_real.iter().any(|&t| t < MIN_EXECUTION_TIME)
        {
            warnings.push(Warnings::FastExecutionTime);
        }

        // Check program exit codes
        if !all_succeeded {
            warnings.push(Warnings::NonZeroExitCode);
        }

        // Run outlier detection
        let scores = modified_zscores(&times_real);

        let outlier_warning_options = OutlierWarningOptions {
            warmup_in_use: self.options.warmup_count > 0,
            prepare_in_use: self
                .options
                .preparation_command
                .as_ref()
                .map(|v| v.len())
                .unwrap_or(0)
                > 0,
        };

        if scores[0] > OUTLIER_THRESHOLD {
            warnings.push(Warnings::SlowInitialRun(
                times_real[0],
                outlier_warning_options,
            ));
        } else if scores.iter().any(|&s| s.abs() > OUTLIER_THRESHOLD) {
            warnings.push(Warnings::OutliersDetected(outlier_warning_options));
        }

        // Warn if poop metrics were requested but not collected
        if self.options.poop_metrics_enabled
            && aggregated_poop_metrics
                .as_ref()
                .is_none_or(|m| !m.has_data())
        {
            warnings.push(Warnings::PoopMetricsUnavailable);
        }

        if !warnings.is_empty() {
            eprintln!(" ");

            for warning in &warnings {
                eprintln!("  {}: {}", "Warning".yellow(), warning);
            }
        }

        if self.options.output_style != OutputStyleOption::Disabled {
            println!(" ");
        }

        self.run_cleanup_command(self.command.get_parameters().iter().cloned(), output_policy)?;

        // Collect poop metrics
        let poop_metrics_all: Vec<PoopMetrics> = timing_results
            .iter()
            .filter_map(|tr| tr.poop_metrics)
            .collect();
        let poop_metrics_all = if poop_metrics_all.is_empty() {
            None
        } else {
            Some(poop_metrics_all)
        };
        let poop_metrics = aggregated_poop_metrics;

        Ok(BenchmarkResult {
            command: self.command.get_name(),
            command_with_unused_parameters: self.command.get_name_with_unused_parameters(),
            mean: t_mean,
            stddev: t_stddev,
            median: t_median,
            user: user_mean,
            system: system_mean,
            min: t_min,
            max: t_max,
            times: Some(times_real),
            memory_usage_byte: Some(memory_usage_byte),
            exit_codes,
            parameters: self
                .command
                .get_parameters()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            poop_metrics,
            poop_metrics_all,
        })
    }
}
