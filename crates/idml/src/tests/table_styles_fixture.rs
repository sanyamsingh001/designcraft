// Shared synthetic IDML input for importer and engine/native-format regressions.
// This module is included only by tests; all content is hand-written.
use std::io::Write;

pub fn fixture(explicit_styles: bool, local_overrides: bool) -> Vec<u8> {
    fixture_with_regions(explicit_styles, local_overrides, [false; 4])
}

/// Same-as-body flags in header, footer, left-column, right-column order.
pub fn fixture_with_regions(explicit_styles: bool, local_overrides: bool, same_as_body: [bool; 4]) -> Vec<u8> {
    fixture_with_regions_and_local_cell(explicit_styles, local_overrides.then_some((1, 1)), same_as_body)
}

pub fn fixture_with_regions_and_local_cell(explicit_styles: bool, local_cell: Option<(usize, usize)>, same_as_body: [bool; 4]) -> Vec<u8> {
    const DESIGNMAP: &str = r#"<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging"
        Self="d" StoryList="s1" Name="Table style fixture">
      <idPkg:Graphic src="Resources/Graphic.xml"/>
      <idPkg:Styles src="Resources/Styles.xml"/>
      <Layer Self="L1" Name="Art" Visible="true" Locked="false"/>
      <idPkg:Spread src="Spreads/Spread_sp1.xml"/>
      <idPkg:Story src="Stories/Story_s1.xml"/>
    </Document>"#;
    const GRAPHIC: &str = r#"<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
      <Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>
      <Color Self="Color/Paper" Model="Process" Space="CMYK" ColorValue="0 0 0 0" Name="Paper"/>
      <Color Self="Color/Brand" Model="Process" Space="RGB" ColorValue="255 0 0" Name="Brand"/>
      <Swatch Self="Swatch/None" Name="None"/>
    </idPkg:Graphic>"#;
    const SPREAD: &str = r#"<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
      <Spread Self="sp1">
        <Page Self="p1" Name="1" GeometricBounds="0 0 300 300" ItemTransform="1 0 0 1 0 0"/>
        <TextFrame Self="tf1" ParentStory="s1" ItemLayer="L1" ItemTransform="1 0 0 1 0 0"
            PreviousTextFrame="n" NextTextFrame="n">
          <Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray>
            <PathPointType Anchor="20 20"/><PathPointType Anchor="20 280"/>
            <PathPointType Anchor="280 280"/><PathPointType Anchor="280 20"/>
          </PathPointArray></GeometryPathType></PathGeometry></Properties>
        </TextFrame>
      </Spread>
    </idPkg:Spread>"#;
    let mut styles = String::from(
        r#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
        <RootParagraphStyleGroup Self="rp">
          <ParagraphStyle Self="ParagraphStyle/CellText" Name="CellText" PointSize="10"/>
          <ParagraphStyle Self="ParagraphStyle/Local" Name="Local" PointSize="14"/>
        </RootParagraphStyleGroup><RootCellStyleGroup Self="rc">"#,
    );
    for (id, name, fill, tint, insets, vj, bottom, color, kind) in [
        ("uHead", "Header", "Color/Black", 20, [6, 7, 8, 9], "CenterAlign", 0.5, "Color/Black", "$ID/Solid"),
        ("uBody", "Body row", "Swatch/None", 100, [2, 3, 4, 5], "TopAlign", 0.5, "Color/Black", "$ID/Solid"),
        ("uFoot", "Footer", "Color/Black", 60, [3, 4, 5, 6], "BottomAlign", 0.5, "Color/Black", "$ID/Solid"),
        ("uLeft", "Pastille", "Color/Brand", 100, [4, 8, 4, 8], "CenterAlign", 0.5, "Color/Black", "$ID/Solid"),
        ("uRight", "Right", "Color/Paper", 75, [5, 6, 7, 8], "BottomAlign", 0.5, "Color/Black", "$ID/Solid"),
        ("uExplicit", "Explicit", "Color/Brand", 40, [11, 12, 13, 14], "CenterAlign", 0.75, "Color/Brand", "$ID/Dotted"),
    ] {
        styles.push_str(&format!(
            r#"<CellStyle Self="CellStyle/{id}" Name="{name}" FillColor="{fill}" FillTint="{tint}"
              TopInset="{}" LeftInset="{}" BottomInset="{}" RightInset="{}"
              VerticalJustification="{vj}" AppliedParagraphStyle="ParagraphStyle/CellText"
              TopEdgeStrokeWeight="0" TopEdgeStrokeColor="Color/Black"
              LeftEdgeStrokeWeight="0" LeftEdgeStrokeColor="Swatch/None"
              BottomEdgeStrokeWeight="{bottom}" BottomEdgeStrokeColor="{color}" BottomEdgeStrokeTint="100"
              BottomEdgeStrokeType="{kind}" RightEdgeStrokeWeight="0" RightEdgeStrokeColor="Swatch/None"/>"#,
            insets[0], insets[1], insets[2], insets[3],
        ));
    }
    styles.push_str(&format!(
        r#"</RootCellStyleGroup><RootTableStyleGroup Self="rt">
        <TableStyle Self="TableStyle/uCatalogue" Name="Catalogue"
          TopBorderStrokeWeight="0" LeftBorderStrokeWeight="0" BottomBorderStrokeWeight="0" RightBorderStrokeWeight="0"
          TopBorderStrokeColor="Color/Black" TopBorderStrokeTint="50"
          HeaderRegionCellStyle="CellStyle/uHead" BodyRegionCellStyle="CellStyle/uBody"
          FooterRegionCellStyle="CellStyle/uFoot" LeftColumnRegionCellStyle="CellStyle/uLeft"
          RightColumnRegionCellStyle="CellStyle/uRight" HeaderRegionSameAsBodyRegion="{header}"
          FooterRegionSameAsBodyRegion="{footer}" LeftColumnRegionSameAsBodyRegion="{left}"
          RightColumnRegionSameAsBodyRegion="{right}" SpaceBefore="5" SpaceAfter="6"/>
        </RootTableStyleGroup></idPkg:Styles>"#,
        header = same_as_body[0],
        footer = same_as_body[1],
        left = same_as_body[2],
        right = same_as_body[3],
    ));
    let table_local = if local_cell.is_some() {
        r#" TopBorderStrokeWeight="2" TopBorderStrokeColor="Color/Brand" TopBorderStrokeTint="80" SpaceBefore="9""#
    } else {
        ""
    };
    let mut story = format!(
        r#"<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
        <Story Self="s1"><ParagraphStyleRange><CharacterStyleRange>
        <Table AppliedTableStyle="TableStyle/uCatalogue" HeaderRowCount="1" FooterRowCount="1"{table_local}>
          <Row Name="0" MinimumHeight="40"/><Row Name="1" MinimumHeight="40"/><Row Name="2" MinimumHeight="40"/>
          <Column Name="0" SingleColumnWidth="60"/><Column Name="1" SingleColumnWidth="60"/><Column Name="2" SingleColumnWidth="60"/>"#,
    );
    for row in 0..3 {
        for col in 0..3 {
            let style = match (row, col) {
                (0, _) => "uHead",
                (2, _) => "uFoot",
                (1, 0) => "uLeft",
                (1, 2) => "uRight",
                _ => "uExplicit",
            };
            let applied = if explicit_styles { format!(r#" AppliedCellStyle="CellStyle/{style}""#) } else { String::new() };
            let is_local = local_cell == Some((row, col));
            let cell_local = if is_local {
                r#" FillColor="Color/Paper" FillTint="35" TopInset="12" VerticalJustification="BottomAlign"
                TopEdgeStrokeColor="Swatch/None" TopEdgeStrokePriority="2" LeftEdgeStrokePriority="0"
                BottomEdgeStrokeWeight="2" BottomEdgeStrokeColor="Color/Brand" BottomEdgeStrokeTint="60""#
            } else {
                ""
            };
            let para_local = if is_local { r#" AppliedParagraphStyle="ParagraphStyle/Local""# } else { "" };
            story.push_str(&format!(
                r#"<Cell Name="{col}:{row}" BottomEdgeStrokePriority="1"{applied}{cell_local}>
                <ParagraphStyleRange{para_local}><CharacterStyleRange><Content>{row}:{col}</Content>
                </CharacterStyleRange></ParagraphStyleRange></Cell>"#,
            ));
        }
    }
    story.push_str("</Table></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>");
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/vnd.adobe.indesign-idml-package").unwrap();
    for (name, content) in [
        ("designmap.xml", DESIGNMAP),
        ("Resources/Graphic.xml", GRAPHIC),
        ("Resources/Styles.xml", styles.as_str()),
        ("Spreads/Spread_sp1.xml", SPREAD),
        ("Stories/Story_s1.xml", story.as_str()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
