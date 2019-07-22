#!/usr/bin/env python3

import os
import numpy as np
import json
import sys
import typing as tp
from scipy import interpolate as scint
from scipy import sparse
import argparse
from pymatgen import Structure

import unfold_lib

THZ_TO_WAVENUMBER = 33.3564095198152

try:
    import rsp2
except ImportError:
    info = lambda s: print(s, file=sys.stderr)
    info('Please add the following to your PYTHONPATH:')
    info('  (rsp2 source root)/scripts')
    info('  (rsp2 source root)/src/python')
    sys.exit(1)

from rsp2.io import eigensols, structure_dir, dwim

DEFAULT_TOL = 1e-2

A = tp.TypeVar('A')
B = tp.TypeVar('B')

def main():
    global SHOW_ACTION_STACK

    parser = argparse.ArgumentParser(
        description="Unfold phonon eigenvectors",
        epilog=
            'Uses the method of P. B. Allen et al., "Recovering hidden Bloch character: '
            'Unfolding electrons, phonons, and slabs", Phys Rev B, 87, 085322.',
        )

    parser.add_argument('--verbose', action='store_true')
    parser.add_argument('--debug', action='store_true')
    parser.add_argument('STRUCTURE', help='rsp2 structure directory')

    all_tasks = []
    def register(task):
        nonlocal all_tasks

        all_tasks.append(task)
        return task

    # Considering that all of the constructor args are explicitly type-annotated,
    # and that IntelliJ has no problem here telling you when you have the wrong
    # number of arguments, you would think that IntelliJ should be able to tell
    # you when you mix up the order of two of the arguments.
    #
    # You would be wrong.  Hence all the keyword arguments.

    structure = register(TaskStructure())

    kpoint_sfrac = register(TaskKpointSfrac())

    dynmat = register(TaskDynmat())

    eigensols = register(TaskEigensols(structure=structure, dynmat=dynmat))

    translation_deperms = register(TaskDeperms(structure=structure))

    ev_gpoint_probs = register(TaskGProbs(structure=structure, kpoint_sfrac=kpoint_sfrac, eigensols=eigensols, translation_deperms=translation_deperms))

    band_path = register(TaskBandPath(structure=structure))

    mode_data = register(TaskEigenmodeData(eigensols=eigensols))

    raman_json = register(TaskRamanJson())

    multi_qpoint_data = register(TaskMultiQpointData(mode_data=mode_data, kpoint_sfrac=kpoint_sfrac, ev_gpoint_probs=ev_gpoint_probs))

    band_qg_indices = register(TaskBandQGIndices(structure=structure, multi_qpoint_data=multi_qpoint_data, band_path=band_path))

    _bandplot = register(TaskBandPlot(band_path=band_path, band_qg_indices=band_qg_indices, raman_json=raman_json, multi_qpoint_data=multi_qpoint_data))

    for task in all_tasks:
        task.add_parser_opts(parser)

    args = parser.parse_args()

    if not any(task.has_action(args) for task in all_tasks):
        parser.error("Nothing to do!")

    if args.debug:
        SHOW_ACTION_STACK = True

    for task in all_tasks:
        task.check_upfront(args)

    for task in all_tasks:
        if task.has_action(args):
            task.require(args)

#----------------------------------------------------------------
# CLI logic deciding when to compute certain things or e.g. to read files.
#
# Written in a vaguely declarative style with the help of a Task class
# that defers computation until it is needed.

# FIXME remove globals
ACTION_STACK = []
SHOW_ACTION_STACK = False

T = tp.TypeVar
class Task:
    NOT_YET_COMPUTED = object()

    def __init__(self):
        self.cached = Task.NOT_YET_COMPUTED

    def add_parser_opts(self, parser: argparse.ArgumentParser):
        pass

    def check_upfront(self, args):
        pass

    def has_action(self, args):
        return False

    def require(self, args):
        """ Force computation of the task, and immediately perform any actions
        associated with it (e.g. writing a file).

        It is cached after the first call so that it need not be run again.
        """
        global ACTION_STACK

        if self.cached is Task.NOT_YET_COMPUTED:
            ACTION_STACK.append(type(self).__name__)
            self.cached = self._compute(args)
            ACTION_STACK.pop()
            self._do_action(args)

        return self.cached

    def _compute(self, args):
        raise NotImplementedError

    def _do_action(self, args):
        """ A task performed after """
        pass

class TaskKpointSfrac(Task):
    def add_parser_opts(self, parser):
        parser.add_argument(
            '--kpoint', type=type(self).parse, help=
            'Q-point in fractional coordinates of the superstructure reciprocal '
            'cell, as a whitespace-separated list of 3 integers, floats, or '
            'rational numbers.',
        )

    def _compute(self, args):
        return list(args.kpoint)

    @classmethod
    def parse(cls, s):
        """ Can be used by other tasks to replicate the behavior of --kpoint. """
        return parse_kpoint(s)

class TaskRamanJson(Task):
    def add_parser_opts(self, parser):
        parser.add_argument(
            '--raman-file', help=
            'rsp2 raman.json output file. Required if colorizing a plot by raman.',
        )

    def check_upfront(self, args):
        check_optional_input(args.raman_file)

    def _compute(self, args):
        if not args.raman_file:
            die('--raman-file is required for this action')

        return dwim.from_path(args.raman_file)

class TaskStructure(Task):
    def add_parser_opts(self, parser):
        # Arguments related to layer projections
        parser.add_argument(
            '--layer', type=int, default=0,
            help=
            'The output will be in the BZ of this layer (indexed from 0).',
        )

        parser.add_argument(
            '--layer-mode', choices=['one'],
            help=
            '--layer-mode=one will only consider the projection of the '
            'eigenvectors onto the primary layer, meaning the total norm of '
            'some eigenvectors in the output may be less than 1. (or even =0)',
        )

    def _compute(self, args, **_kw):
        layer = args.layer

        if not os.path.isdir(args.STRUCTURE):
            die('currently, only rsp2 structure directory format is supported')

        sdir = structure_dir.from_path(args.STRUCTURE)
        structure = sdir.structure
        if sdir.layer_sc_matrices is None:
            die("the structure must supply layer-sc-matrices")
        supercell = Supercell(sdir.layer_sc_matrices[args.layer])

        # Project everything onto a single layer
        if sdir.layers is None:
            mask = np.array([1] * len(structure))
        else:
            mask = np.array(sdir.layers) == layer

        projected_structure = Structure(
            structure.lattice,
            np.array(structure.species)[mask],
            structure.frac_coords[mask],
        )

        return {
            'supercell': supercell,
            'layer': layer,
            'mask': mask,
            'structure': structure,
            'projected_structure': projected_structure,
        }

class TaskDeperms(Task):
    def __init__(self, structure: TaskStructure):
        super().__init__()
        self.structure = structure

    def add_parser_opts(self, parser):
        parser.add_argument(
            '--write-perms', metavar='FILE', help=
            'Write permutations of translations to this file. (.npy, .npy.xz)',
        )

        parser.add_argument(
            '--perms', metavar='FILE', help=
            'Path to file previously written through --write-perms.',
        )

    def check_upfront(self, args):
        check_optional_input(args.perms)
        check_optional_output_ext('--write-perms', args.write_perms, forbid='.npz')

    def has_action(self, args):
        return bool(args.write_perms)

    def _compute(self, args):
        if args.perms:
            return np.array(dwim.from_path(args.perms))
        else:
            progress_callback = None
            if args.verbose:
                def progress_callback(done, count):
                    print(f'Deperms: {done:>5} of {count} translations')

            return collect_translation_deperms(
                superstructure=self.structure.require(args)['projected_structure'],
                supercell=self.structure.require(args)['supercell'],
                axis_mask=np.array([1,1,0]),
                tol=DEFAULT_TOL,
                progress=progress_callback,
            )

    def _do_action(self, args):
        if args.write_perms:
            translation_deperms = self.require(args)
            dwim.to_path(args.write_perms, translation_deperms)

class TaskDynmat(Task):
    def add_parser_opts(self, parser):
        parser.add_argument(
            '--dynmat', metavar='FILE', help='rsp2 dynmat file (.npz)',
        )

    def check_upfront(self, args):
        check_optional_input(args.dynmat)

    def _compute(self, args):
        if not args.dynmat:
            die('--dynmat is required for this action')
        return sparse.load_npz(args.dynmat)

class TaskEigensols(Task):
    def __init__(self, structure: TaskStructure, dynmat: TaskDynmat):
        super().__init__()
        self.structure = structure
        self.dynmat = dynmat

    def add_parser_opts(self, parser):
        parser.add_argument(
            '--eigensols', metavar='FILE',
            help='read rsp2 eigensols file. (.npz)',
        )

        parser.add_argument(
            '--write-eigensols', metavar='FILE',
            help='write rsp2 eigensols file. (.npz)',
        )

    def check_upfront(self, args):
        check_optional_input(args.eigensols)
        check_optional_output_ext('--write-eigensols', args.write_eigensols, forbid='.npy')

    def has_action(self, args):
        return bool(args.write_eigensols)

    def _compute(self, args):
        mask = self.structure.require(args)['mask']
        nsites = len(mask)

        if args.eigensols:
            if args.verbose:
                # This file can be very large and reading it can take a long time
                print('Reading eigensols file')

            ev_eigenvalues, ev_eigenvectors = eigensols.from_path(args.eigensols)

        else:
            import scipy.linalg
            if args.verbose:
                print('--eigensols not supplied. Will diagonalize dynmat.')

            m = self.dynmat.require(args)
            if np.all(m.data.imag == 0.0):
                m = m.real
            ev_eigenvalues, ev_eigenvectors = scipy.linalg.eigh(m.todense())
            ev_eigenvectors = ev_eigenvectors.T

        ev_projected_eigenvectors = ev_eigenvectors.reshape((-1, nsites, 3))[:, mask]

        return {
            'ev_eigenvalues': ev_eigenvalues,
            'ev_eigenvectors': ev_eigenvectors,
            'ev_projected_eigenvectors': ev_projected_eigenvectors,
        }

    def _do_action(self, args):
        if args.write_eigensols:
            d = self.require(args)
            esols = d['ev_eigenvalues'], d['ev_eigenvectors']
            eigensols.to_path(args.write_eigensols, esols)

class TaskEigenmodeData(Task):
    """ Scalar data about eigenmodes for the plot. """
    def __init__(self, eigensols: TaskEigensols):
        super().__init__()
        self.eigensols = eigensols

    def add_parser_opts(self, parser):
        parser.add_argument(
            '--write-mode-data', metavar='FILE', help=
            'Write data about plotted eigenmodes to this file. (.npz)',
        )

        parser.add_argument(
            '--mode-data', metavar='FILE', help=
            'Read data previously written using --write-mode-data so that reading '
            'the (large) eigensols file is not necessary to produce a plot.',
        )

    def check_upfront(self, args):
        check_optional_input(args.mode_data)
        check_optional_output_ext('--write-mode-data', args.write_mode_data, forbid='.npy')

    def has_action(self, args):
        return bool(args.write_mode_data)

    def _compute(self, args):
        if args.mode_data:
            return type(self).read_file(args.mode_data)

        if args.verbose:
            print('--mode-data not supplied. Computing from eigensols.')

        ev_eigenvalues = self.eigensols.require(args)['ev_eigenvalues']
        ev_eigenvectors = self.eigensols.require(args)['ev_eigenvectors']

        ev_frequencies = eigensols.wavenumber_from_eigenvalue(ev_eigenvalues)

        ev_z_coords = ev_eigenvectors.reshape((-1, ev_eigenvectors.shape[1]//3, 3))[:, :, 2]
        ev_z_projections = np.linalg.norm(ev_z_coords, axis=1)**2
        return {
            'ev_frequencies': ev_frequencies,
            'ev_z_projections': ev_z_projections,
        }

    def _do_action(self, args):
        if args.write_mode_data:
            d = self.require(args)
            np.savez_compressed(
                args.write_mode_data,
                ev_frequencies=d['ev_frequencies'],
                ev_z_projections=d['ev_z_projections'],
            )

    @classmethod
    def read_file(cls, path):
        """ Can be used by other tasks to replicate the behavior of --mode-data. """
        npz = np.load(path)
        return {
            'ev_frequencies': npz.f.ev_frequencies,
            'ev_z_projections': npz.f.ev_z_projections,
        }

# Arguments related to probabilities
# (an intermediate file format that can be significantly smaller than the
#  input eigenvectors and thus easier to transmit)
class TaskGProbs(Task):
    def __init__(
            self,
            structure: TaskStructure,
            kpoint_sfrac: TaskKpointSfrac,
            eigensols: TaskEigensols,
            translation_deperms: TaskDeperms,
    ):
        super().__init__()
        self.structure = structure
        self.kpoint_sfrac = kpoint_sfrac
        self.eigensols = eigensols
        self.translation_deperms = translation_deperms

    def add_parser_opts(self, parser):
        parser.add_argument(
            '--write-probs', metavar='FILE', help=
            'Write magnitudes of g-point projections to this file. (.npz)',
        )

        parser.add_argument(
            '--probs-threshold', type=float, default=1e-7, help=
            'Truncate probabilities smaller than this when writing probs. '
            'This can significantly reduce disk usage.',
        )

        parser.add_argument(
            '--probs-impl', choices=['auto', 'rust', 'python'], default='auto', help=
            'Enable the experimental rust unfolder.',
        )

        parser.add_argument(
            '--probs', metavar='FILE', help=
            'Path to .npz file previously written through --write-probs.',
        )

    def check_upfront(self, args):
        check_optional_input(args.probs)
        check_optional_output_ext('--write-probs', args.write_probs, forbid='.npy')

        if args.probs_impl in ['rust', 'auto']:
            try:
                unfold_lib.build()
            except unfold_lib.BuildError:
                assert unfold_lib.unfold_all is None
                if args.probs_impl == 'rust':
                    raise

    def has_action(self, args):
        return bool(args.write_probs)

    def _compute(self, args):
        if args.probs:
            return type(self).read_file(args.probs, args)
        else:
            if args.verbose:
                print('--probs not supplied. Will compute by unfolding eigensols.')

            layer = self.structure.require(args)['layer']

            progress_prefix = f'Layer {layer}: ' if args.verbose else None

            # reading the file might take forever; compute deperms first as it has
            # a greater chance of having a bug
            self.translation_deperms.require(args)

            ev_gpoint_probs = unfold_all(
                superstructure=self.structure.require(args)['projected_structure'],
                supercell=self.structure.require(args)['supercell'],
                eigenvectors=self.eigensols.require(args)['ev_projected_eigenvectors'],
                kpoint_sfrac=self.kpoint_sfrac.require(args),
                translation_deperms=self.translation_deperms.require(args),
                implementation=args.probs_impl,
                progress_prefix=progress_prefix,
            )
            return type(self).__postprocess(ev_gpoint_probs, args)

    def _do_action(self, args):
        ev_gpoint_probs = self.require(args)
        if args.write_probs:
            dwim.to_path(args.write_probs, ev_gpoint_probs)

    @classmethod
    def read_file(cls, path, args):
        """ Can be used by other tasks to replicate the behavior of --probs. """
        ev_gpoint_probs = dwim.from_path(path)
        return cls.__postprocess(ev_gpoint_probs, args)

    @classmethod
    def __postprocess(cls, ev_gpoint_probs, args):
        if args.verbose:
            debug_bin_magnitudes(ev_gpoint_probs)

        ev_gpoint_probs = truncate(ev_gpoint_probs, args.probs_threshold)
        ev_gpoint_probs = sparse.csr_matrix(ev_gpoint_probs)

        if args.verbose:
            density = ev_gpoint_probs.nnz / product(ev_gpoint_probs.shape)
            print('Probs matrix density: {:.4g}%'.format(100.0 * density))

        return ev_gpoint_probs

class TaskBandPath(Task):
    def __init__(
            self,
            structure: TaskStructure,
    ):
        super().__init__()
        self.structure = structure

    def add_parser_opts(self, parser):
        parser.add_argument(
            '--plot-kpath', help=
            "A kpath in the format accepted by ASE's parse_path_string, "
            "naming points in the monolayer BZ.  If not specified, no band "
            "plot is generated."
        )

    def _compute(self, args):
        from ase.dft.kpoints import bandpath, parse_path_string

        supercell = self.structure.require(args)['supercell']
        super_lattice = self.structure.require(args)['structure'].lattice.matrix

        prim_lattice = np.linalg.inv(supercell.matrix) @ super_lattice

        if args.plot_kpath is None:
            die('--plot-kpath is required')

        # NOTE: The kpoints returned by get_special_points (and by proxy, this
        #       function) do adapt to the user's specific choice of primitive cell.
        #       (at least, for reasonable cells; I haven't tested it with a highly
        #       skewed cell). Respect!
        bandpath_output = bandpath(args.plot_kpath, prim_lattice, 300)

        point_names = parse_path_string(args.plot_kpath)
        if len(point_names) > 1:
            die('This script currently does not support plots along discontinuous paths.')
        point_names, = point_names

        point_names = [r'$\mathrm{\Gamma}$' if x == 'G' else x for x in point_names]

        return {
            'plot_kpoint_pfracs': bandpath_output[0],
            'plot_x_coordinates': bandpath_output[1],
            'plot_xticks': bandpath_output[2],
            'plot_xticklabels': point_names,
        }

class TaskMultiQpointData(Task):
    def __init__(
            self,
            mode_data: TaskEigenmodeData,
            kpoint_sfrac: TaskKpointSfrac,
            ev_gpoint_probs: TaskGProbs,
    ):
        super().__init__()
        self.mode_data = mode_data
        self.kpoint_sfrac = kpoint_sfrac
        self.ev_gpoint_probs = ev_gpoint_probs

    def add_parser_opts(self, parser):
        parser.add_argument(
            '--multi-qpoint-file', help=
            "Multi-kpoint manifest file.  This allows using data from multiple "
            "kpoints to be included on a single plot. If this is supplied, many "
            "arguments for dealing with a single kpoint (e.g. --dynmat, --kpoint) "
            f"will be ignored.\n\n{MULTI_KPOINT_FILE_HELP_STR}"
        )

    def check_upfront(self, args):
        check_optional_input(args.multi_qpoint_file)

    def _compute(self, args):
        if args.multi_qpoint_file:
            return type(self).read_file(args.multi_qpoint_file, args)
        else:
            return type(self).__process_dicts({
                "qpoint-sfrac": self.kpoint_sfrac.require(args),
                "mode-data": self.mode_data.require(args),
                "probs": self.ev_gpoint_probs.require(args),
            })

    @classmethod
    def read_file(cls, path, args):
        d = dwim.from_path(path)
        if not isinstance(d, list):
            die(f'Expected {path} to contain a sequence/array.')

        base_dir = os.path.dirname(path)
        rel_path = lambda name: os.path.join(base_dir, name)

        dicts = []
        unrecognized_keys = set()
        for item in d:
            dicts.append({
                "qpoint-sfrac": TaskKpointSfrac.parse(item.pop('kpoint')),
                "mode-data": TaskEigenmodeData.read_file(rel_path(item.pop('mode-data'))),
                "probs": TaskGProbs.read_file(rel_path(item.pop('probs')), args),
            })
            unrecognized_keys.update(item)

        if unrecognized_keys:
            warn(f"Unrecognized keys in multi-kpoint manifest: {repr(sorted(unrecognized_keys))}")

        return cls.__process_dicts(*dicts)

    @classmethod
    def __process_dicts(cls, *dicts):
        dict_of_lists = dict_zip(*dicts)
        dict_of_lists['mode-data'] = dict_zip(*dict_of_lists['mode-data'])
        dict_of_lists['num-qpoints'] = len(dict_of_lists['qpoint-sfrac'])
        return dict_of_lists

MULTI_KPOINT_FILE_KEYS = ["kpoint", "probs", "mode-data"]

MULTI_KPOINT_FILE_HELP_STR = f"""
The multi-kpoint manifest is a sequence (encoded in JSON or YAML) whose elements
are mappings with the keys: {repr(MULTI_KPOINT_FILE_KEYS)}. Each of these keys
maps to a string exactly like the corresponding CLI argument.  This means that
in order to use this option, you will first need to generate files at each
Q-point in individual runs using --write-probs and --write-mode-data.
""".strip().replace('\n', ' ')

# Performs resampling along the high symmetry path.
class TaskBandQGIndices(Task):
    def __init__(
            self,
            structure: TaskStructure,
            band_path: TaskBandPath,
            multi_qpoint_data: TaskMultiQpointData,
    ):
        super().__init__()
        self.structure = structure
        self.band_path = band_path
        self.multi_qpoint_data = multi_qpoint_data

    def _compute(self, args):
        return resample_qg_indices(
                super_lattice=self.structure.require(args)['structure'].lattice.matrix,
                supercell=self.structure.require(args)['supercell'],
                qpoint_sfrac=self.multi_qpoint_data.require(args)['qpoint-sfrac'],
                plot_kpoint_pfracs=self.band_path.require(args)['plot_kpoint_pfracs'],
        )

class TaskBandPlot(Task):
    def __init__(
            self,
            band_path: TaskBandPath,
            band_qg_indices: TaskBandQGIndices,
            raman_json: TaskRamanJson,
            multi_qpoint_data: TaskMultiQpointData,
    ):
        super().__init__()
        self.band_path = band_path
        self.band_qg_indices = band_qg_indices
        self.raman_json = raman_json
        self.multi_qpoint_data = multi_qpoint_data

    def add_parser_opts(self, parser):
        parser.add_argument('--show', action='store_true', help='show plot')
        parser.add_argument('--write-plot', metavar='FILE', help='save plot to file')

        parser.add_argument(
            '--plot-exponent', type=float, metavar='VALUE', default=1.0, help=
            'Scale probabilities by this exponent before plotting.'
        )

        parser.add_argument(
            '--plot-max-alpha', type=float, metavar='VALUE', default=1.0, help=
            'Scale probabilities by this exponent before plotting.'
        )

        parser.add_argument(
            '--plot-truncate', type=float, metavar='VALUE', default=0.0, help=
            'Don\'t plot points whose final alpha is less than this. '
            'This can be a good idea for SVG and PDF outputs.'
        )

        parser.add_argument(
            '--plot-baseline-file', type=str, metavar='FILE', help=
            'Data file for a "normal" plot.  Phonopy band.yaml is accepted.'
        )

        parser.add_argument(
            '--plot-color', type=str, default='zpol', metavar='SCHEME', help=
            'How the plot points are colored. Choices: [zpol, sign, uniform:COLOR, raman:POL] '
            '(e.g. --plot-color uniform:blue). POL is either "average-3d" or "backscatter".'
        )

        parser.add_argument(
            '--plot-sidebar', action='store_true', help=
            'Show a sidebar with the frequencies all on the same point.'
        )

        parser.add_argument(
            '--plot-ylim', help=
            'Set plot ylim.'
        )

        parser.add_argument(
            '--plot-hide-unfolded', action='store_true', help=
            "Don't actually show the unfolded probs. (intended for use with --plot-baseline-file, "
            "so that you can show only the baseline)"
        )

    def has_action(self, args):
        return args.show or bool(args.write_plot)

    def _compute(self, args):
        multi_qpoint_data = self.multi_qpoint_data.require(args)

        raman_dict = None
        if args.plot_color.startswith('raman:'):
            if multi_qpoint_data['num-qpoints'] == 1:
                # make the dict items indexed by [qpoint (just the one)][ev]
                raman_dict = self.raman_json.require(args)
                raman_dict = { k: np.array([v]) for (k, v) in raman_dict.items() }
            else:
                warn('raman coloring cannot be used with multiple kpoints')
                args.plot_color = 'zpol'

        mode_data = multi_qpoint_data['mode-data']
        q_ev_gpoint_probs = np.array(multi_qpoint_data['probs'])

        if args.plot_sidebar and len(multi_qpoint_data) > 1:
            warn("--plot-sidebar doesn't make sense with multiple kpoints")

        return probs_to_band_plot(
            q_ev_frequencies=np.array(mode_data['ev_frequencies']),
            q_ev_z_projections=np.array(mode_data['ev_z_projections']),
            q_ev_gpoint_probs=q_ev_gpoint_probs ,
            path_g_indices=self.band_qg_indices.require(args)['G'],
            path_q_indices=self.band_qg_indices.require(args)['Q'],
            path_x_coordinates=self.band_path.require(args)['plot_x_coordinates'],
            plot_xticks=self.band_path.require(args)['plot_xticks'],
            plot_xticklabels=self.band_path.require(args)['plot_xticklabels'],
            alpha_exponent=args.plot_exponent,
            alpha_max=args.plot_max_alpha,
            alpha_truncate=args.plot_truncate,
            raman_dict=raman_dict,
            plot_baseline_path=args.plot_baseline_file,
            plot_color=args.plot_color,
            plot_ylim=args.plot_ylim,
            plot_sidebar=args.plot_sidebar,
            plot_hide_unfolded=args.plot_hide_unfolded,
            verbose=args.verbose,
        )

    def _do_action(self, args):
        fig, ax = self.require(args)

        if args.write_plot:
            fig.savefig(args.write_plot)

        if args.show:
            import matplotlib.pyplot as plt
            # fig.show() # doesn't do anything :/
            plt.show()

#----------------------------------------------------------------

# Here we encounter a big problem:
#
#     The points at which we want to draw bands are not necessarily
#     images of the qpoint at which we computed eigenvectors.
#
# Our solution is not very rigorous. For each point on the plot's x-axis, we
# will simply produce the projected probabilities onto the nearest image of
# the supercell qpoint.
#
# The idea is that for large supercells, every point in the primitive BZ
# is close to an image of the supercell qpoint point. (though this scheme
# may fail to produce certain physical effects that are specifically enabled
# by the symmetry of a high-symmetry point when that point is not an image
# of the qpoint)
#
# With the addition of --multi-kpoint-file, the density of the points we
# are sampling from can be even further increased.
def resample_qg_indices(
        super_lattice,
        supercell,
        qpoint_sfrac,
        plot_kpoint_pfracs,
):
    gpoint_sfracs = supercell.gpoint_sfracs()

    sizes = check_arrays(
        super_lattice = (super_lattice, [3, 3], np.floating),
        gpoint_sfracs = (gpoint_sfracs, ['quotient', 3], np.floating),
        qpoint_sfrac = (qpoint_sfrac, ['qpoint', 3], np.floating),
        plot_kpoint_pfracs = (plot_kpoint_pfracs, ['plot-x', 3], np.floating),
    )

    prim_lattice = np.linalg.inv(supercell.matrix) @ super_lattice

    # All of the (Q + G) points at which probabilities were computed.
    qg_sfracs = np.vstack([
        supercell.gpoint_sfracs() + sfrac
        for sfrac in qpoint_sfrac
    ])
    qg_carts = qg_sfracs @ np.linalg.inv(super_lattice).T
    assert qg_carts.shape == (sizes['qpoint'] * sizes['quotient'], 3)

    # For each of those (Q + G) points, the index of its Q point and its K point.
    qg_q_ids, qg_g_ids = np.mgrid[0:sizes['qpoint'], 0:sizes['quotient']].reshape((2, -1))

    # For every point on the plot x-axis, the index of the closest Q + G point
    plot_kpoint_carts = plot_kpoint_pfracs @ np.linalg.inv(prim_lattice).T
    plot_kpoint_qg_ids = griddata_periodic(
        points=qg_carts,
        values=np.arange(sizes['qpoint'] * sizes['quotient']),
        xi=plot_kpoint_carts,
        lattice=np.linalg.inv(prim_lattice).T,
        periodic_axis_mask=[1,1,0],
        method='nearest',
    )

    # For every plot on the plot x-axis, the indices of Q and G for the
    # nearest (Q + G)
    return {
        'Q': qg_q_ids[plot_kpoint_qg_ids],
        'G': qg_g_ids[plot_kpoint_qg_ids],
    }

def probs_to_band_plot(
        q_ev_frequencies,
        q_ev_z_projections,
        q_ev_gpoint_probs,
        path_g_indices,
        path_q_indices,
        path_x_coordinates,
        plot_xticks,
        plot_xticklabels,
        plot_color,
        plot_ylim,
        raman_dict,
        alpha_truncate,
        alpha_exponent,
        alpha_max,
        plot_baseline_path,
        plot_hide_unfolded,
        plot_sidebar,
        verbose=False,
):
    import matplotlib.pyplot as plt

    set_plot_color, q_ev_extra = get_plot_color_setter(plot_color, z_pol=q_ev_z_projections, raman_dict=raman_dict)

    sizes = check_arrays(
        q_ev_frequencies = (q_ev_frequencies, ['qpoint', 'ev'], np.floating),
        q_ev_z_projections = (q_ev_z_projections, ['qpoint', 'ev'], np.floating),
        q_ev_gpoint_probs = (q_ev_gpoint_probs, ['qpoint'], object),
        q_ev_gpoint_probs_row = (q_ev_gpoint_probs[0], ['ev', 'quotient'], np.floating),
        path_g_indices = (path_g_indices, ['plot-x'], np.integer),
        path_q_indices = (path_q_indices, ['plot-x'], np.integer),
        path_x_coordinates = (path_x_coordinates, ['plot-x'], np.floating),
        plot_xticks = (plot_xticks, ['special_point'], np.floating),
    )
    if raman_dict is not None:
        assert raman_dict['average-3d'].shape == (sizes['qpoint'], sizes['ev'])

    def get_ev_path_probs():
        # If ev_gpoint_probs were a dense, 3d array, we could just write
        # ev_gpoint_probs[path_q_indices, :, path_g_indices]... but because
        # there are sparse matrices in there we must do this.
        q_g_ev_probs = np.array([
            list(ev_g_probs.T)
            for ev_g_probs in q_ev_gpoint_probs
        ])
        assert q_g_ev_probs.shape == (sizes['qpoint'], sizes['quotient']), (q_g_ev_probs.shape, (sizes['qpoint'], sizes['quotient']))
        assert sparse.issparse(q_g_ev_probs[0][0])

        path_ev_probs = sparse.vstack(q_g_ev_probs[path_q_indices, path_g_indices])
        return np.asarray(path_ev_probs.todense()) # it's no longer N^2 but rather X*N

    path_ev_probs = get_ev_path_probs()
    path_ev_frequencies = q_ev_frequencies[path_q_indices]
    path_ev_extra = q_ev_extra[path_q_indices]
    assert path_ev_probs.shape == (sizes['plot-x'], sizes['ev'])
    assert path_ev_frequencies.shape == (sizes['plot-x'], sizes['ev'])
    assert path_ev_extra.shape == (sizes['plot-x'], sizes['ev'])

    if plot_baseline_path is not None:
        base_X, base_Y = read_baseline_plot(plot_baseline_path)
    else:
        base_X, base_Y = [], []

    X, Y, S, Extra = [], [] ,[], []
    iterator = zip(path_x_coordinates, path_ev_frequencies, path_ev_probs, path_ev_extra)
    for (x_coord, ev_frequencies, ev_probs, ev_extra) in iterator:
        # Don't ask matplotlib to draw thousands of points with alpha=0
        mask = ev_probs != 0

        X.append([x_coord] * mask.sum())
        Y.append(ev_frequencies[mask])
        S.append(ev_probs[mask])
        Extra.append(ev_extra[mask])

    X = np.hstack(X)
    Y = np.hstack(Y)
    S = np.hstack(S)
    Extra = np.hstack(Extra)

    S **= alpha_exponent
    S *= alpha_max

    mask = S > alpha_truncate
    X = X[mask]
    Y = Y[mask]
    S = S[mask]
    Extra = Extra[mask]

    if verbose:
        print(f'Plotting {len(X)} points!')

    C = np.hstack([np.zeros((len(S), 3)), S[:, None]])
    set_plot_color(C, X=X, Y=Y, Extra=Extra)

    fig = plt.figure(figsize=(7, 8), constrained_layout=True)
    #fig.set_tight_layout(True)

    if plot_sidebar:
        gs = fig.add_gridspec(ncols=8, nrows=1)
        ax = fig.add_subplot(gs[0,:-1])
        ax_sidebar = fig.add_subplot(gs[0,-1], sharey=ax)
    else:
        ax = fig.add_subplot(111)

    if not plot_hide_unfolded:
        ax.scatter(X, Y, 20, C)
    if plot_baseline_path is not None:
        base_X /= np.max(base_X)
        base_X *= np.max(X)
        ax.scatter(base_X, base_Y, 5, 'k')

    for x in plot_xticks:
        ax.axvline(x, color='k')

    ax.set_xlim(X.min(), X.max())
    ax.set_xticks(plot_xticks)
    ax.set_xticklabels(plot_xticklabels, fontsize=20)
    ax.set_ylabel('Frequency (cm$^{-1}$)', fontsize=20)
    for tick in ax.yaxis.get_major_ticks():
        tick.label.set_fontsize(16)

    if plot_ylim is not None:
        ymin, ymax = (float(x.strip()) for x in plot_ylim.split(':'))
        ax.set_ylim(ymin, ymax)

    if plot_sidebar:
        ax_sidebar.set_xlim(-1, 1)
        ax_sidebar.hlines(Y, -1, 1, color=C)
        ax_sidebar.set_xticks([0])
        ax_sidebar.set_xticklabels([r'$\mathrm{\Gamma}$'], fontsize=20)
        plt.setp(ax_sidebar.get_yticklabels(), visible=False)

    return fig, ax

def get_plot_color_setter(plot_color, z_pol, raman_dict):
    from matplotlib import colors, cm

    # Switch based on plot_color so we can validate it before doing anything expensive.
    extra = np.zeros_like(z_pol)

    if plot_color == 'zpol':
        # colorize Z projection
        extra = z_pol
        def set_plot_color(C, *, Extra, **_kw):
            cmap = colors.LinearSegmentedColormap.from_list('', [[0, 0, 1], [0, 0.5, 0]])
            C[:, :3] = cmap(Extra)[:, :3]

    elif plot_color == 'sign':
        def set_plot_color(C, *, Y, **_kw):
            C[:,:3] = colors.to_rgb('y')
            C[:,:3] = np.where((Y < -1e-3)[:, None], colors.to_rgb('g'), C[:,:3])
            C[:,:3] = np.where((Y > +1e-3)[:, None], colors.to_rgb('r'), C[:,:3])

    elif plot_color.startswith('uniform:'):
        fixed_color = colors.to_rgb(plot_color[len('uniform:'):].strip())
        def set_plot_color(C, **_kw):
            # use given color
            C[:, :3] = fixed_color

    elif plot_color.startswith('raman:'):
        raman_dict_key = plot_color[len('raman:'):]
        if raman_dict_key not in ['average-3d', 'backscatter']:
            die('Invalid raman polarization mode: {raman_dict_key}')
        extra = raman_dict[raman_dict_key]

        def set_plot_color(C, *, Extra, **_kw):
            Extra = np.log(np.maximum(Extra.max() * 1e-7, Extra))
            Extra -= Extra.min()
            Extra /= Extra.max()

            cmap = cm.get_cmap('cool_r')
            C[:, :3] = cmap(Extra)[:, :3]

    else:
        die(f'invalid --plot-color: {repr(plot_color)}')
        raise RuntimeError('unreachable')

    return set_plot_color, extra

def read_baseline_plot(path):
    X, Y = [], []
    d = dwim.from_path(path)
    d = d['phonon']
    for qpoint in d:
        X.extend([qpoint['distance']] * len(qpoint['band']))
        Y.extend(band['frequency'] * THZ_TO_WAVENUMBER for band in qpoint['band'])
    return X, Y

def reduce_carts(carts, lattice):
    fracs = carts @ np.linalg.inv(lattice)
    fracs %= 1.0
    fracs %= 1.0 # for values like -1e-20
    return fracs @ lattice

class Supercell:
    def __init__(self, matrix):
        """
        :param matrix: Shape ``(3, 3)``, integer.
        Integer matrix satisfying
        ``matrix @ prim_lattice_matrix == super_lattice_matrix``
        where the lattice matrices are understood to store a lattice primitive
        translation in each row.
        """
        if isinstance(matrix, Supercell):
            self.matrix = matrix.matrix
            self.repeats = matrix.repeats
            self.t_repeats = matrix.t_repeats
        else:
            matrix = np.array(matrix, dtype=int)
            assert matrix.shape == (3, 3)
            self.matrix = matrix
            self.repeats = find_repeats(matrix)
            self.t_repeats = find_repeats(matrix.T)

    def translation_pfracs(self):
        """
        :return: Shape ``(quotient, 3)``, integral.

        Fractional coordinates of quotient-space translations,
        in units of the primitive cell lattice basis vectors.
        """
        return cartesian_product(*(np.arange(n) for n in self.repeats)).astype(float)

    def gpoint_sfracs(self):
        """
        :return: Shape ``(quotient, 3)``, integral.

        Fractional coordinates of quotient-space gpoints,
        in units of the supercell reciprocal lattice basis vectors.
        """
        return cartesian_product(*(np.arange(n) for n in self.t_repeats)).astype(float)

def collect_translation_deperms(
        superstructure: Structure,
        supercell: Supercell,
        axis_mask = np.array([True, True, True]),
        tol: float = DEFAULT_TOL,
        progress = None,
):
    """
    :param superstructure:
    :param supercell:

    :param axis_mask: Shape ``(3,)``, boolean.
    Permutation finding can be troublesome if the primitive cell translational
    symmetry is very strongly broken along some axis (e.g. formation of ripples
    in a sheet of graphene).  This can be used to filter those axes out of these
    permutation searches.

    :param tol: ``float``.
    Cartesian distance within which sites must overlap to be considered
    equivalent.

    :param progress: Progress callback.
    Called as ``progress(num_done, num_total)``.
    :return:
    """
    super_lattice = superstructure.lattice.matrix
    prim_lattice = np.linalg.inv(supercell.matrix) @ super_lattice

    # The quotient group of primitive lattice translations modulo the supercell
    translation_carts = supercell.translation_pfracs() @ prim_lattice

    # debug_quotient_points(translation_carts[:, :2], super_lattice[:2,:2])

    # Allen's method requires the supercell to approximately resemble the
    # primitive cell, so that translations of the eigenvector can be emulated
    # by permuting its data.
    return list(map_with_progress(
        translation_carts, progress,
        lambda translation_cart: get_translation_deperm(
            structure=superstructure,
            translation_cart=translation_cart,
            axis_mask=axis_mask,
            tol=tol,
        ),
    ))

def unfold_all(
        superstructure: Structure,
        supercell: Supercell,
        eigenvectors,
        kpoint_sfrac,
        translation_deperms,
        implementation,
        progress_prefix = None,
):
    """
    :param superstructure: ``pymatgen.Structure`` object with `sites` sites.
    :param supercell: ``Supercell`` object.
    :param eigenvectors: Shape ``(num_evecs, 3 * sites)``, complex or real.

    Each row is an eigenvector.  Their norms may be less than 1, if the
    structure has been projected onto a single layer, but should not exceed 1.
    (They will NOT be automatically normalized by this function, as projection
    onto a layer may create eigenvectors of zero norm)

    :param translation_deperms:  Shape ``(quotient, sites)``.
    Permutations such that ``(carts + translation_carts[i])[deperms[i]]`` is
    equivalent to ``carts`` under superlattice translational symmetry, where
    ``carts`` is the supercell carts.

    :param kpoint_sfrac: Shape ``(3,)``, real.
    The K point in the SC reciprocal cell at which the eigenvector was computed,
    in fractional coords.

    :return: Shape ``(num_evecs, quotient)``
    For each vector in ``eigenvectors``, its projected probabilities
    onto ``k + g`` for each g in ``supercell.gpoint_sfracs()``.
    """
    super_lattice = superstructure.lattice.matrix
    prim_lattice = np.linalg.inv(supercell.matrix) @ super_lattice

    # The quotient group of primitive lattice translations modulo the supercell
    translation_carts = supercell.translation_pfracs() @ prim_lattice
    translation_sfracs = translation_carts @ np.linalg.inv(super_lattice)

    # debug_quotient_points(translation_carts[:, :2], super_lattice[:2,:2])

    # The quotient group of supercell reciprocal lattice points modulo the
    # primitive reciprocal cell
    gpoint_sfracs = supercell.gpoint_sfracs()

    # debug_quotient_points((gpoint_sfracs @ np.linalg.inv(super_lattice).T)[:, :2], np.linalg.inv(prim_lattice).T[:2,:2])

    super_lattice_recip = superstructure.lattice.inv_matrix.T
    kpoint_cart = kpoint_sfrac @ super_lattice_recip
    super_carts = superstructure.cart_coords
    translation_phases = np.vstack([
        get_translation_phases(
            kpoint_cart=kpoint_cart,
            super_carts=super_carts,
            translation_cart=translation_cart,
            translation_deperm=translation_deperm,
        )
        for (translation_cart, translation_deperm)
        in zip(translation_carts, translation_deperms)
    ])

    #----------------------------
    # Rust impl

    if unfold_lib.unfold_all is not None and implementation != 'python':
        return unfold_lib.unfold_all(
            superstructure=superstructure,
            translation_carts=translation_carts,
            gpoint_sfracs=gpoint_sfracs,
            kpoint_sfrac=kpoint_sfrac,
            eigenvectors=eigenvectors,
            translation_deperms=translation_deperms,
            translation_phases=translation_phases,
            progress_prefix=progress_prefix,
        )

    #----------------------------
    # Python impl

    progress = None
    if progress_prefix is not None:
        def progress(done, count):
            print(f'{progress_prefix}Unfolding {done:>5} of {count} eigenvectors')

    return np.array(list(map_with_progress(
        eigenvectors, progress,
        lambda eigenvector: unfold_one(
            translation_sfracs=translation_sfracs,
            translation_deperms=translation_deperms,
            translation_phases=translation_phases,
            gpoint_sfracs=gpoint_sfracs,
            kpoint_sfrac=kpoint_sfrac,
            eigenvector=eigenvector.reshape((-1, 3)),
        )
    )))

def unfold_one(
        translation_sfracs,
        translation_deperms,
        translation_phases,
        gpoint_sfracs,
        kpoint_sfrac,
        eigenvector,
):
    """
    :param translation_sfracs: Shape ``(quotient, 3)``, real.
    The quotient space translations (PC lattice modulo super cell),
    in units of the supercell basis vectors.

    :param translation_deperms: Shape ``(quotient, sc_sites)``, integral.
    Permutations such that ``(carts + translation_carts[i])[deperms[i]]``
    is ordered like the original carts. (i.e. when applied to the coordinate
    data, it translates by ``-1 * translation_carts[i]``)

    For any vector of per-site metadata ``values``, ``values[deperms[i]]`` is
    effectively translated by ``translation_carts[i]``.

    :param translation_phases: Shape ``(quotient, sc_sites)``, complex.
    The phase factors that must be factored into each atom's components after
    a translation to account for the fact that permuting the sites does not
    produce the same images of sites as actually translating them would.

    :param gpoint_sfracs: Shape ``(quotient, 3)``, real.
    Translations in the reciprocal quotient space, in units of the reciprocal
    lattice basis vectors.

    (SC reciprocal lattice modulo primitive BZ)

    :param kpoint_sfrac: Shape ``(3,)``, real.
    The K point in the SC reciprocal cell at which the eigenvector was computed.

    :param eigenvector: Shape ``(sc_sites, 3)``, complex.
    A normal mode of the supercell. (arbitrary norm)

    :return: Shape ``(quotient,)``, real.
    Probabilities of `eigenvector` projected onto each kpoint
    ``kpoint + qpoints[i]``.
    """

    translation_sfracs = np.array(translation_sfracs)
    translation_deperms = np.array(translation_deperms)
    gpoint_sfracs = np.array(gpoint_sfracs)
    kpoint_sfrac = np.array(kpoint_sfrac)
    eigenvector = np.array(eigenvector)
    sizes = check_arrays(
        translation_sfracs = (translation_sfracs, ['quotient', 3], np.floating),
        translation_deperms = (translation_deperms, ['quotient', 'sc_sites'], np.integer),
        gpoint_sfracs = (gpoint_sfracs, ['quotient', 3], np.floating),
        kpoint_sfrac = (kpoint_sfrac, [3], np.floating),
        eigenvector = (eigenvector, ['sc_sites', 3], [np.floating, np.complexfloating]),
    )

    inner_prods = np.array([
        np.vdot(eigenvector, t_phases[:, None] * eigenvector[t_deperm])
        for (t_deperm, t_phases) in zip(translation_deperms, translation_phases)
    ])

    gpoint_probs = []
    for g in gpoint_sfracs:
        # SBZ kpoint dot r for every r
        k_dot_rs = (kpoint_sfrac + g) @ translation_sfracs.T
        phases = np.exp(-2j * np.pi * k_dot_rs)

        prob = sum(inner_prods * phases) / sizes['quotient']

        # analytically, these are all real, positive numbers
        assert abs(prob.imag) < 1e-7
        assert -1e-7 < prob.real
        gpoint_probs.append(max(prob.real, 0.0))
    gpoint_probs = np.array(gpoint_probs)

    np.testing.assert_allclose(gpoint_probs.sum(), np.linalg.norm(eigenvector)**2, atol=1e-7)
    return gpoint_probs

def get_translation_deperm(
        structure: Structure,
        translation_cart,
        axis_mask = np.array([1, 1, 1]),
        tol: float = DEFAULT_TOL,
):
    # NOTE: Heavily-optimized function for identifying permuted structures.
    #       Despite the name, it works just as well for translations as it does
    #       for rotations.
    # FIXME: Shouldn't be relying on this
    from phonopy.structure.cells import compute_permutation_for_rotation

    lattice = structure.lattice.matrix
    fracs_original = structure.frac_coords
    fracs_translated = (structure.cart_coords + translation_cart) @ np.linalg.inv(lattice)

    # Possibly ignore displacements along certain axes.
    fracs_original *= axis_mask
    fracs_translated *= axis_mask

    # Compute the inverse permutation on coordinates, which is the
    # forward permutation on metadata ("deperm").
    #
    # I.e. ``fracs_translated[deperm] ~~ fracs_original``
    return compute_permutation_for_rotation(
        fracs_translated, fracs_original, lattice, tol,
    )

# When we apply the translation operators, some atoms will map to images under
# the supercell that are different from the ones we have eigenvector data for.
# For kpoints away from supercell gamma, those images should have different
# phases in their eigenvector components.
#
# Picture that the supercell looks like this:
#
# Legend:
# - a diagram of integers (labeled "Indices") depicts coordinates, by displaying
#   the number `i` at the position of the `i`th atom.
# - a diagram with letters depicts a list of metadata (such as elements or
#   eigenvector components) by arranging them starting from index 0 in the lower
#   left, and etc. as if they were to label the original coords.
# - Parentheses surround the position of the original zeroth atom.
#
#                 6  7  8                    g  h  i
#    Indices:    3  4  5     Eigenvector:   d  e  f
#              (0) 1  2                   (a) b  c
#
# Consider the translation that moves the 0th atom to the location originally at
# index 3. Applying the deperm to the eigenvector (to "translate" it by this
# vector) yields:
#
#                       d  e  f
#      Eigenvector:    a  b  c
#                    (g) h  i
#
# In this example, g, h, and i do not have the correct phases because those
# atoms mapped to different images. To find the superlattice translation that
# describes these images, we must look at the coords.  First, translate the
# coords by literally applying the translation.  Then, apply the inverse coperm
# to make the indices match their original sites.
#
#   (applying translation...)      (...then applying inverse coperm)
#                  6  7  8                     0  1  2
#                 3  4  5                     6  7  8
#    Indices:    0  1  2         Indices:    3  4  5
#              (x) x  x                    (x) x  x
#
# If you subtract the original coordinates from these, you get a list of
# metadata describing the super-lattice translations for each site in the
# permuted structure; atoms 3..9 need no correction, while atoms 0..3 require a
# phase correction by some super-lattice vector R.
#
#                       0  0  0
#    Image vectors:    0  0  0
#                    (R) R  R
#
def get_translation_phases(
        kpoint_cart,
        super_carts,
        translation_cart,
        translation_deperm,
):
    inverse_coperm = translation_deperm # inverse of inverse

    # translate, permute, and subtract to get superlattice points
    image_carts = (super_carts + translation_cart)[inverse_coperm] - super_carts

    # dot each atom's R with the kpoint to produce its phase correction
    return np.exp(2j * np.pi * image_carts @ kpoint_cart)

def find_repeats(supercell_matrix):
    """
    Get the number of distinct translations along each lattice primitive
    translation. (it's the diagonal of the row-based HNF of the matrix)

    :param supercell_matrix: Shape ``(3, 3)``, integer.
    Integer matrix satisfying
    ``matrix @ prim_lattice_matrix == super_lattice_matrix``
    where the lattice matrices are understood to store a lattice primitive
    translation in each row.

    :return:
    """
    from abelian import hermite_normal_form
    from sympy import Matrix

    expected_volume = abs(round(np.linalg.det(supercell_matrix)))

    supercell_matrix = Matrix(supercell_matrix) # to sympy

    # abelian.hermite_normal_form is column-based, so give it the transpose
    hnf = hermite_normal_form(supercell_matrix.T)[1].T
    hnf = np.array(hnf).astype(int) # to numpy

    assert round(np.linalg.det(hnf)) == expected_volume
    return np.diag(hnf)

def griddata_periodic(
        points,
        values,
        xi,
        lattice,
        # set this to reduce memory overhead
        periodic_axis_mask=(1,1,1),
        **kwargs,
):
    """
    scipy.interpolate.griddata, but where points (in cartesian) are periodic
    with the given lattice.  The data provided is complemented by images
    from the surrounding unit cells.

    The lattice is assumed to have small skew.
    """
    points_frac = reduce_carts(points, lattice) @ np.linalg.inv(lattice)
    xi_frac = reduce_carts(xi, lattice) @ np.linalg.inv(lattice)
    #debug_path((points_frac @ lattice)[:,:2], lattice[:2,:2], (xi_frac @ lattice)[:,:2])

    for axis, mask_bit in enumerate(periodic_axis_mask):
        if mask_bit:
            unit = [0] * 3
            unit[axis] = 1

            points_frac = np.vstack([
                points_frac - unit,
                points_frac,
                points_frac + unit,
                ])
            values = np.hstack([values] * 3)

    points = points_frac @ lattice
    xi = xi_frac @ lattice

    # Delete axes in which the points have no actual extent, because
    # they'll make QHull mad. (we'd be giving it a degenerate problem)
    true_axis_mask = [1, 1, 1]
    for axis in reversed(range(3)):
        max_point = points_frac[:, axis].max()
        if np.allclose(max_point, points_frac[:, axis].min()):
            np.testing.assert_allclose(max_point, xi_frac[:, axis].min())
            np.testing.assert_allclose(max_point, xi_frac[:, axis].max())

            xi = np.delete(xi, axis, axis=1)
            points = np.delete(points, axis, axis=1)
            true_axis_mask[axis] = 0

    #if xi.shape[1] == 2:
    #    debug_path(points, lattice, xi)

    return scint.griddata(points, values, xi, **kwargs)

def truncate(array, tol):
    array = array.copy()
    if sparse.issparse(array):
        data = array.data
        data[np.absolute(data) < tol] = 0.0
        return array
    else:
        array[np.absolute(array) < tol] = 0.0
        return array

def debug_bin_magnitudes(array):
    from collections import Counter

    zero_count = product(array.shape) - np.sum(array != 0)
    if sparse.issparse(array):
        array = array.data
    array = array[array != 0]
    logs = np.floor(np.log10(array)).astype(int)
    counter = Counter(logs)
    counter[-999] = zero_count
    print("Magnitude summary:")
    print(sorted(counter.items()))

def product(iter):
    from functools import reduce
    return reduce((lambda a, b: a * b), iter)

#---------------------------------------------------------------
# CLI types

def parse_kpoint(s):
    def parse_number(word):
        try:
            if '/' in word:
                numer, denom = (int(x.strip()) for x in word.split('/'))
                return numer / denom
            else:
                return float(word.strip())
        except ValueError:
            raise ValueError(f'{repr(word)} is not an integer, float, or rational number')

    if '[' in s:
        warn('JSON input for --kpoint is deprecated; use a whitespace separated list of numbers.')
        lst = [1.0 * x for x in json.loads(s)]
    else:
        lst = [parse_number(word) for word in s.split()]

    if len(lst) != 3:
        raise ValueError('--kpoint must be of dimension 3')

    return lst

#---------------------------------------------------------------
# debugging

def debug_quotient_points(points2, lattice2):
    import matplotlib.pyplot as plt

    fig, ax = plt.subplots(figsize=(7, 8))
    draw_unit_cell(ax, lattice2, lw=2)
    draw_reduced_points(ax, points2, lattice2)
    ax.set_aspect('equal', 'box')
    plt.show()

def debug_path(points, lattice, path):
    import matplotlib.pyplot as plt
    from matplotlib.path import Path
    import matplotlib.patches as patches

    lattice = lattice[:, :2] # FIXME
    lattice_path = Path(
        vertices=[[0,0], lattice[0,], lattice[0]+lattice[1], lattice[1], [0,0]],
        codes = [Path.MOVETO] + [Path.LINETO] * 4,
    )
    mpl_path = Path(
        vertices=path,
        codes = [Path.MOVETO] + [Path.LINETO] * (len(path) - 1),
    )

    fig, ax = plt.subplots(figsize=(7, 8))
    ax.scatter(points[:, 0], points[:, 1])
    ax.add_patch(patches.PathPatch(lattice_path, facecolor='none', lw=2))
    ax.add_patch(patches.PathPatch(mpl_path, facecolor='none', ls=':', lw=1))
    plt.show()

def draw_path(ax, path, **kw):
    from matplotlib.path import Path
    import matplotlib.patches as patches

    path = np.array(path)
    print(path.shape)
    mpl_path = Path(
        vertices=path,
        codes=[Path.MOVETO] + [Path.LINETO] * (len(path) - 1),
    )
    ax.add_patch(patches.PathPatch(mpl_path, facecolor='none', **kw))

def draw_unit_cell(ax, lattice2, **kw):
    np.testing.assert_equal(lattice2.shape, [2,2])
    path = [[0,0], lattice2[0], lattice2[0]+lattice2[1], lattice2[1], [0,0]]
    draw_path(ax, path, **kw)

def draw_reduced_points(ax, points2, lattice2, **kw):
    points2 = reduce_carts(points2, lattice2)
    ax.scatter(points2[:, 0], points2[:, 1], **kw)

def check_arrays(**kw):
    previous_values = {}

    kw = {name: list(data) for name, data in kw.items()}
    for name in kw:
        if not sparse.issparse(kw[name][0]):
            kw[name][0] = np.array(kw[name][0])

    for name, data in kw.items():
        if len(data) == 2:
            array, dims = data
            dtype = None
        elif len(data) == 3:
            array, dims, dtype = data
        else:
            raise TypeError(f'{name}: Expected (array, shape) or (array, shape, dtype)')

        if dtype:
            # support one dtype or a list of them
            if type(dtype) is type:
                dtype = [dtype]
            if not any(issubclass(np.dtype(array.dtype).type, d) for d in dtype):
                raise TypeError(f'{name}: Expected one of {dtype}, got {array.dtype}')

        # format names without quotes
        nice_expected = '[' + ', '.join(map(str, dims)) + ']'
        if len(dims) != array.ndim:
            raise TypeError(f'{name}: Wrong number of dimensions (expected shape {nice_expected}, got {list(array.shape)})')

        for axis, dim in enumerate(dims):
            if isinstance(dim, int):
                if array.shape[axis] != dim:
                    raise TypeError(f'{name}: Mismatched dimension (expected shape {nice_expected}, got {list(array.shape)})')
            elif isinstance(dim, str):
                if dim not in previous_values:
                    previous_values[dim] = (array.shape[axis], name, axis)

                if previous_values[dim][0] != array.shape[axis]:
                    prev_value, prev_name, prev_axis = previous_values[dim]
                    raise TypeError(
                        f'Conflicting values for dimension {repr(dim)}:\n'
                        f' {prev_name}: {kw[prev_name][0].shape} (axis {prev_axis}: {prev_value})\n'
                        f' {name}: {array.shape} (axis {axis}: {array.shape[axis]})'
                    )

    return {dim:tup[0] for (dim, tup) in previous_values.items()}

#---------------------------------------------------------------

def check_optional_input(path):
    if path is not None and not os.path.exists(path):
        die(f'Does not exist: \'{path}\'')

def check_optional_output_ext(argument, path, only=None, forbid=None):
    """ Validate the extension for an output file.

    Because this script uses DWIM facilities, some arguments support many possible
    filetypes.  However, there are some cases where it's easy to forget whether
    something should be .npy or .npz.  Calling this function with `forbid=` in this
    case can be helpful.
    """
    if path is None:
        return

    if only is None and forbid is None:
        raise TypeError('must supply only or forbid')

    if forbid is not None:
        if type(forbid) is str:
            forbid = [forbid]

        for ext in forbid:
            if path.endswith(ext) or path.endswith(ext + '.gz') or path.endswith(ext + '.xz'):
                die(f'Invalid extension for {argument}: {path}')

    if only is not None:
        if type(only) is str:
            only = [only]

        if not any(path.endswith(ext) for ext in only):
            expected = ', '.join(only)
            die(f'Invalid extension for {argument}: expected one of: {expected}')

#---------------------------------------------------------------
# utils

def map_with_progress(
        xs: tp.Iterator[A],
        progress: tp.Callable[[int, int], None],
        function: tp.Callable[[A], B],
) -> tp.Iterator[B]:
    yield from (function(x) for x in iter_with_progress(xs, progress))

def iter_with_progress(
        xs: tp.Iterator[A],
        progress: tp.Callable[[int, int], None],
) -> tp.Iterator[A]:
    xs = list(xs)

    for (num_done, x) in enumerate(xs):
        if progress:
            progress(num_done, len(xs))

        yield x

    if progress:
        progress(len(xs), len(xs))

def cartesian_product(*arrays):
    la = len(arrays)
    dtype = np.result_type(*arrays)
    arr = np.empty([len(a) for a in arrays] + [la], dtype=dtype)
    for i, a in enumerate(np.ix_(*arrays)):
        arr[..., i] = a
    return arr.reshape(-1, la)

def dict_zip(*dicts):
    """
    Take a series of dicts that share the same keys, and reduce the values
    for each key as if folding an iterator.
    """
    keyset = set(dicts[0])
    for d in dicts:
        if set(d) != keyset:
            raise KeyError(f"Mismatched keysets in fold_dicts: {sorted(keyset)}, {sorted(set(d))}")

    return { key: [d[key] for d in dicts] for key in keyset }

def warn(*args, **kw):
    print('unfold:', *args, **kw, file=sys.stderr)

def die(*args, **kw):
    warn(*args, **kw)
    if SHOW_ACTION_STACK:
        for name in ACTION_STACK[::-1]:
            print(f"  while computing {name}", file=sys.stderr)
    sys.exit(1)

#---------------------------------------------------------------

if __name__ == '__main__':
    main()
