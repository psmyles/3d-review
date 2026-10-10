# Reports

**Copy summary** puts a short plain-text summary on the clipboard: the file,
the profile, how many checks found errors, warnings and info, and one line per
finding. It is meant for pasting into a ticket or a chat.

**Save report** writes every finding to a JSON file a pipeline can read. Parts
are named as they are in the file, faces are numbered by their polygon index
within their mesh and points by their control-point index, the way your
modelling program numbers them, so a report can be followed back to the
source. Long lists are cut short in the file, but the counts stay exact.

The report also carries the profile it was checked against, so it says which
rules it applied.
