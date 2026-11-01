"""Check the formatting conventions enforced by the iOS swift-format configuration."""

from pathlib import Path
import subprocess
import unittest


CONFIGURATION = Path(__file__).resolve().parents[1] / ".swift-format"


class SwiftFormatTests(unittest.TestCase):
    def format(self, source):
        command = ["swift", "format"]
        if CONFIGURATION.exists():
            command.extend(["--configuration", str(CONFIGURATION)])
        result = subprocess.run(
            [*command, "-"], input=source, text=True, capture_output=True, check=True
        )
        return result.stdout

    def test_wrapped_arguments_each_have_their_own_line(self):
        result = self.format(
            'let result = makeVoyage(title: "A voyage with a sufficiently long title", '
            'description: "A description that exceeds the column limit", granularity: .km1)\n'
        )
        self.assertIn('\n  title: "A voyage with a sufficiently long title",\n', result)
        self.assertIn('\n  description: "A description that exceeds the column limit",\n', result)
        self.assertIn("\n  granularity: .km1\n", result)

    def test_wrapped_generic_requirements_each_have_their_own_line(self):
        result = self.format(
            "func compare<FirstCollection, SecondCollection>() "
            "where FirstCollection: Collection, SecondCollection: Collection, "
            "FirstCollection.Element: Equatable, "
            "FirstCollection.Element == SecondCollection.Element {}\n"
        )
        self.assertIn("FirstCollection: Collection,\n", result)
        self.assertIn("SecondCollection: Collection,\n", result)

    def test_parameterized_attributes_precede_the_declaration(self):
        result = self.format('@available(iOS 18, *) func openVoyage() {}\n')
        self.assertEqual(result, '@available(iOS 18, *)\nfunc openVoyage() {}\n')

    def test_imports_are_sorted_and_testable_imports_are_separate(self):
        result = self.format(
            "@testable import PinVoyagerDomain\nimport Testing\nimport Foundation\n"
        )
        self.assertEqual(
            result,
            "import Foundation\nimport Testing\n\n@testable import PinVoyagerDomain\n",
        )

    def test_member_spacing_survives_and_extra_blank_lines_collapse(self):
        result = self.format(
            "struct Voyage {\n    let title: String\n\n\n"
            "    func save() {}\n\n    func publish() {}\n}\n"
        )
        self.assertEqual(
            result,
            "struct Voyage {\n  let title: String\n\n"
            "  func save() {}\n\n  func publish() {}\n}\n",
        )

    def test_multiline_collections_have_trailing_commas(self):
        result = self.format('let titles = [\n  "First voyage",\n  "Second voyage"\n]\n')
        self.assertIn('"Second voyage",\n]', result)


if __name__ == "__main__":
    unittest.main()
